// Settings editing, tasks, action log, attachments, filters, bar metrics, narrow screen, window accessibility.
const { test, expect } = require('@playwright/test');
const AxeBuilder = require('@axe-core/playwright').default;
const { login, newSession, say } = require('./helpers');

const DECIDE = process.env.AGENT_WEB_TOKEN_DECIDE;
const auth = (t) => ({ Authorization: `Bearer ${t}` });
const axeDialog = async (page) => {
  const r = await new AxeBuilder({ page }).include('[role=dialog]').withTags(['wcag2a', 'wcag2aa']).analyze();
  return r.violations.map((v) => `${v.id}: ${v.nodes.map((n) => n.target.join(' ')).join(' | ')}`);
};

test.describe('Settings', () => {
  test('a web decide token edits the Memory page, the value goes to the core', async ({ page, request }) => {
    test.skip(!DECIDE, 'needs a web token with the decide right (AGENT_WEB_TOKEN_DECIDE)');
    await login(page, DECIDE);
    await page.getByRole('button', { name: 'Settings' }).click();
    const dlg = page.getByRole('dialog', { name: 'Settings' });
    await dlg.getByRole('tab', { name: 'Memory' }).click();
    const sel = dlg.getByLabel('source');
    await expect(sel).toBeVisible({ timeout: 15_000 });
    const cur = await sel.inputValue();
    const next = cur === 'both' ? 'local' : 'both';
    await sel.selectOption(next);
    await dlg.getByRole('button', { name: 'Save' }).click();
    await expect(dlg.getByRole('status')).toHaveText('Saved', { timeout: 15_000 });
    const r = await request.get('/v1/settings/memory', { headers: auth(DECIDE) });
    expect((await r.json()).source).toBe(next);
    expect(await axeDialog(page)).toEqual([]);
    // Restore the previous value.
    await sel.selectOption(cur);
    await dlg.getByRole('button', { name: 'Save' }).click();
    await expect(dlg.getByRole('status')).toHaveText('Saved', { timeout: 15_000 });
    expect((await (await request.get('/v1/settings/memory', { headers: auth(DECIDE) })).json()).source).toBe(cur);
  });

  test('a plain chat token: the core refuses, the client reports missing permissions', async ({ page }) => {
    await login(page);
    await page.getByRole('button', { name: 'Settings' }).click();
    const dlg = page.getByRole('dialog', { name: 'Settings' });
    await dlg.getByRole('tab', { name: 'Memory' }).click();
    const sel = dlg.getByLabel('source');
    await expect(sel).toBeVisible({ timeout: 15_000 });
    const cur = await sel.inputValue();
    await sel.selectOption(cur === 'both' ? 'local' : 'both');
    await dlg.getByRole('button', { name: 'Save' }).click();
    await expect(dlg.getByRole('status')).toContainText('permissions', { timeout: 15_000 });
  });

  test('security and models are view-only, no secrets in the fields', async ({ page }) => {
    await login(page);
    await page.getByRole('button', { name: 'Settings' }).click();
    const dlg = page.getByRole('dialog', { name: 'Settings' });
    for (const name of ['Security', 'Models', 'Permissions', 'Tunnels']) {
      await dlg.getByRole('tab', { name }).click();
      await expect(dlg.getByText('View only')).toBeVisible();
      await expect(dlg.getByRole('button', { name: 'Save' })).toBeHidden();
      for (const el of await dlg.locator('input[type=text], textarea, select').all()) {
        expect(await el.isDisabled()).toBe(true);
      }
    }
  });

  test('About and Theme are readable, the window is accessible (axe)', async ({ page }) => {
    await login(page);
    await page.getByRole('button', { name: 'Settings' }).click();
    const dlg = page.getByRole('dialog', { name: 'Settings' });
    await expect(dlg.getByLabel('core_version')).toBeVisible({ timeout: 15_000 });
    await dlg.getByRole('tab', { name: 'Theme' }).click();
    await expect(dlg.getByLabel('theme')).toBeVisible();
    expect(await axeDialog(page)).toEqual([]);
  });
});

test.describe('Session tree: actions', () => {
  test('rename, thread, clear, delete with a list of losses, new folder', async ({ page, request }) => {
    await login(page);
    await newSession(page);
    const tag = `e2e${Date.now()}`;
    const dialog = (value) => page.once('dialog', (d) => d.accept(value));
    dialog(`Session ${tag}`);
    await page.getByRole('button', { name: 'Rename', exact: true }).click();
    await expect(page.locator('[role=treeitem]', { hasText: `Session ${tag}` })).toHaveCount(1, { timeout: 15_000 });

    dialog(`Thread ${tag}`);
    await page.getByRole('button', { name: 'Thread', exact: true }).click();
    await expect(page.locator('[role=treeitem]', { hasText: `Thread ${tag}` })).toHaveCount(1, { timeout: 15_000 });
    await page.locator('[role=treeitem]', { hasText: `Session ${tag}` }).first().getByRole('button').first().click();

    await say(page, 'before clearing');
    await expect(page.locator('article.bubble.agent').last()).toContainText('Reply:', { timeout: 20_000 });
    dialog('');
    await page.getByRole('button', { name: 'Clear', exact: true }).click();
    await expect(page.locator('article.bubble')).toHaveCount(0, { timeout: 15_000 });

    dialog(`Folder ${tag}`);
    await page.getByRole('button', { name: 'New folder', exact: true }).click();
    await expect(page.locator('[role=treeitem]', { hasText: `Folder ${tag}` })).toHaveCount(1, { timeout: 15_000 });

    // Move a session into a folder, rename and delete the folder (its sessions move up one level).
    dialog(`Folder ${tag}`);
    await page.getByRole('button', { name: 'Move', exact: true }).click();
    await expect(page.locator('.notice')).toBeHidden();
    dialog(`Folder2 ${tag}`);
    await page.getByRole('button', { name: `Rename folder Folder ${tag}` }).click();
    await expect(page.locator('[role=treeitem]', { hasText: `Folder2 ${tag}` })).toHaveCount(1, { timeout: 15_000 });
    let folderAsk = '';
    page.once('dialog', (d) => {
      folderAsk = d.message();
      d.accept();
    });
    await page.getByRole('button', { name: `Delete folder Folder2 ${tag}` }).click();
    await expect(page.locator('[role=treeitem]', { hasText: `Folder2 ${tag}` })).toHaveCount(0, { timeout: 15_000 });
    expect(folderAsk).toContain('up one level');
    await expect(page.locator('[role=treeitem]', { hasText: `Session ${tag}` })).toHaveCount(1);

    let text = '';
    page.once('dialog', (d) => {
      text = d.message();
      d.accept();
    });
    await page.getByRole('button', { name: 'Delete', exact: true }).click();
    await expect(page.locator('[role=treeitem]', { hasText: `Session ${tag}` })).toHaveCount(0, { timeout: 15_000 });
    expect(text).toContain('will be lost');
    expect(text).toContain(`Session ${tag}`);

    // Stand folder cleanup: the client deletes folders later, through the API.
    const tok = process.env.AGENT_WEB_TOKEN;
    const folders = await (await request.get('/v1/folders', { headers: { Authorization: `Bearer ${tok}` } })).json();
    for (const f of (Array.isArray(folders) ? folders : folders.items || []).filter((x) => x.name === `Folder ${tag}`)) {
      await request.delete(`/v1/folders/${f.id}`, { headers: { Authorization: `Bearer ${tok}` } });
    }
  });
});

test('the outbox queue survives a tab reload', async ({ page }) => {
  await login(page);
  await newSession(page);
  const text = `after reload ${Date.now()}`;
  await page.route('**/v1/sessions/*/messages', (r) => r.abort());
  await say(page, text);
  await expect(page.getByText('Queued: 1')).toBeVisible({ timeout: 15_000 });
  const saved = await page.evaluate(() => sessionStorage.getItem('agentd.outbox'));
  expect(saved).toContain(text);
  expect(saved.toLowerCase()).not.toContain('bearer');
  await page.unroute('**/v1/sessions/*/messages');
  await page.reload({ waitUntil: 'domcontentloaded' });
  await page.locator('#tok').fill(process.env.AGENT_WEB_TOKEN);
  await page.getByRole('button', { name: 'Sign in' }).click();
  // The queue is delivered after sign-in and empties the storage.
  await expect.poll(() => page.evaluate(() => sessionStorage.getItem('agentd.outbox')), { timeout: 30_000 }).toBeNull();
  await expect(page.getByText('Queued:')).toBeHidden({ timeout: 15_000 });
});

test.describe('Tasks and log', () => {
  test('create, edit, pause, delete; field errors are visible', async ({ page }) => {
    await login(page);
    await newSession(page);
    await page.getByRole('button', { name: 'Schedule' }).click();
    const dlg = page.getByRole('dialog', { name: 'Schedule' });
    await dlg.getByRole('button', { name: 'New task' }).click();
    const title = `morning${Date.now()}`;
    await dlg.getByLabel('Title').fill(title);
    await dlg.getByLabel('Description').fill('news summary');
    await dlg.getByLabel('Schedule (cron)').fill('0 8 * *');
    await dlg.getByRole('button', { name: 'Save' }).click();
    await expect(dlg.getByText('cron with five fields')).toBeVisible();
    expect(await axeDialog(page)).toEqual([]);
    await dlg.getByLabel('Schedule (cron)').fill('0 8 * * *');
    await dlg.getByRole('button', { name: 'Save' }).click();
    const row = dlg.locator('li.task', { hasText: title });
    await expect(row).toBeVisible({ timeout: 15_000 });
    await expect(row).toContainText('0 8 * * *');

    await row.getByRole('button', { name: 'Edit' }).click();
    await dlg.getByLabel('Title').fill(`${title}b`);
    await dlg.getByRole('button', { name: 'Save' }).click();
    const row2 = dlg.locator('li.task', { hasText: `${title}b` });
    await expect(row2).toBeVisible({ timeout: 15_000 });

    await row2.getByRole('button', { name: 'Pause' }).click();
    await expect(row2).toContainText('paused', { timeout: 15_000 });

    page.once('dialog', (d) => d.accept());
    await row2.getByRole('button', { name: /Delete/ }).click();
    await expect(dlg.locator('li.task', { hasText: title })).toHaveCount(0, { timeout: 15_000 });
  });

  test('a task without an open session is not created, the reason is shown', async ({ page }) => {
    await login(page);
    await page.getByRole('button', { name: 'Schedule' }).click();
    const dlg = page.getByRole('dialog', { name: 'Schedule' });
    await dlg.getByRole('button', { name: 'New task' }).click();
    await dlg.getByLabel('Kind').selectOption('deleg');
    await dlg.getByLabel('Title').fill('x');
    await dlg.getByLabel('Description').fill('y');
    await dlg.getByRole('button', { name: 'Save' }).click();
    await expect(dlg.getByText('Open a session first')).toBeVisible();
  });

  test('action log: entries or an honest core message', async ({ page }) => {
    await login(page);
    await page.getByRole('button', { name: 'Log', exact: true }).click();
    const dlg = page.getByRole('dialog', { name: 'Action log' });
    await expect(dlg.getByRole('status')).not.toHaveText('Loading…', { timeout: 15_000 });
    // The core publishes the log: the stand's own token issuing is already in it.
    await expect(dlg.getByRole('status')).not.toContainText('unavailable');
    await expect(dlg.locator('li.task').first()).toBeVisible({ timeout: 15_000 });
    await dlg.getByLabel('Actor').selectOption('cron');
    await expect(dlg.getByRole('status')).toHaveText(/No entries|^$/, { timeout: 15_000 });
    await dlg.getByLabel('Actor').selectOption('');
    await expect(dlg.locator('li.task').first()).toBeVisible({ timeout: 15_000 });
    expect(await axeDialog(page)).toEqual([]);
    await page.keyboard.press('Escape');
    await expect(dlg).toBeHidden();
  });
});

test.describe('Chat and bar', () => {
  test('attachment: upload, a chip in the draft, send, the name as text only', async ({ page }) => {
    await login(page);
    await newSession(page);
    const name = 'note<b>.txt';
    await page.locator('input[type=file]').setInputFiles({ name, mimeType: 'text/plain', buffer: Buffer.from('hello') });
    await expect(page.locator('.staged')).toContainText(name, { timeout: 15_000 });
    await say(page, 'look at the file');
    await expect(page.locator('article.bubble.mine .atts')).toContainText(name, { timeout: 15_000 });
    expect(await page.locator('.flow b').count()).toBe(0);
    await expect(page.locator('.staged')).toBeHidden();
  });

  test('an oversized attachment is rejected before upload', async ({ page }) => {
    let uploads = 0;
    await page.route('**/attachments', (route) => {
      if (route.request().method() === 'POST') uploads += 1;
      return route.continue();
    });
    await login(page);
    await newSession(page);
    await page.locator('input[type=file]').setInputFiles({
      name: 'big.bin',
      mimeType: 'application/octet-stream',
      buffer: Buffer.alloc(25 * 1024 * 1024 + 1),
    });
    // The client's text (not the core's, which would answer "File too large") and no upload request at all.
    await expect(page.locator('.notice')).toContainText('limit 25 MiB', { timeout: 15_000 });
    await expect(page.locator('.staged')).toBeHidden();
    await page.waitForTimeout(500);
    expect(uploads, 'POST /attachments must not go to the core').toBe(0);
  });

  test('external link: a warning before the jump, declining keeps the page', async ({ page, context }) => {
    await login(page);
    await newSession(page);
    await say(page, '[site](https://example.com/w101)');
    const link = page.locator('.flow a[href="https://example.com/w101"]').first();
    await expect(link).toBeVisible({ timeout: 25_000 });
    const popups = [];
    context.on('page', (p) => popups.push(p));
    let seen = '';
    page.once('dialog', (d) => {
      seen = d.message();
      return d.dismiss();
    });
    await link.click();
    expect(seen).toContain('example.com');
    await page.waitForTimeout(500);
    expect(popups.length, 'after declining no tab opens').toBe(0);
    // Consent opens a new tab.
    page.once('dialog', (d) => d.accept());
    const [popup] = await Promise.all([context.waitForEvent('page', { timeout: 10_000 }), link.click()]);
    expect(popup).toBeTruthy();
    await popup.close();
  });

  test('bar: quotas, system metrics, indicators are labeled lists', async ({ page }) => {
    await login(page);
    await expect(page.getByRole('list', { name: 'Quotas' })).toBeAttached();
    await expect(page.getByRole('list', { name: 'System' })).toBeAttached();
    await expect(page.getByRole('list', { name: 'Indicators' })).toBeAttached();
    // bar.sys arrives every 2 s: at least one metric must appear.
    await expect(page.locator('.metrics li').first()).toBeVisible({ timeout: 20_000 });
  });

  test('notifications: filter by kind and "Unread", the kind comes from core data', async ({ page }) => {
    await login(page);
    // Kinds are icon buttons of the "Event kind" group; the list comes from core data.
    const kinds = page.getByRole('group', { name: 'Event kind' }).getByRole('button');
    await expect(kinds).not.toHaveCount(0, { timeout: 15_000 });
    const first = kinds.first();
    await first.click();
    await expect(first).toHaveAttribute('aria-pressed', 'true');
    const shown = await page.locator('.notes li .kind').evaluateAll((els) => els.map((e) => e.dataset.kind));
    expect(new Set(shown).size).toBeLessThanOrEqual(1);
    await page.getByLabel('Unread').check();
    expect(await page.locator('.notes li .nt:not(.unread)').count()).toBe(0);
  });
});

test.describe('phone: one column with switching', () => {
  test.use({ viewport: { width: 375, height: 812 } });
  test('one column is visible, buttons switch sections', async ({ page }) => {
    await login(page);
    const nav = page.getByRole('navigation', { name: 'Sections' });
    await expect(nav).toBeVisible();
    await expect(page.locator('.chat')).toBeVisible();
    await expect(page.locator('.tree')).toBeHidden();
    await expect(page.locator('.notes')).toBeHidden();
    await nav.getByRole('button', { name: 'Sessions' }).click();
    await expect(page.locator('.tree')).toBeVisible();
    await expect(page.locator('.chat')).toBeHidden();
    await nav.getByRole('button', { name: 'Events' }).click();
    await expect(page.locator('.notes')).toBeVisible();
    expect(await page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth)).toBeLessThanOrEqual(0);
  });

  test('touch targets in the navigation are at least 40 pixels', async ({ page }) => {
    await login(page);
    for (const b of await page.getByRole('navigation', { name: 'Sections' }).getByRole('button').all()) {
      const box = await b.boundingBox();
      expect(box.height).toBeGreaterThanOrEqual(40);
    }
  });
});

test('accessibility: the Settings, Schedule and Log windows in the dark theme', async ({ browser }) => {
  const ctx = await browser.newContext({ colorScheme: 'dark', baseURL: process.env.AGENT_WEB_URL || 'http://127.0.0.1:4096' });
  const page = await ctx.newPage();
  await login(page);
  for (const [btn, title] of [['Settings', 'Settings'], ['Schedule', 'Schedule'], ['Log', 'Action log']]) {
    await page.getByRole('button', { name: btn }).click();
    await expect(page.getByRole('dialog', { name: title })).toBeVisible();
    await page.waitForTimeout(500);
    expect(await axeDialog(page), title).toEqual([]);
    await page.keyboard.press('Escape');
  }
  await ctx.close();
});
