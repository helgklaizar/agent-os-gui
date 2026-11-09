// Look checks: four columns, a bar, icons instead of text buttons.
// Screenshots (optional): AGENT_WEB_SHOTS=<directory> saves narrow and wide screen shots.
const { test, expect } = require('@playwright/test');
const { login, newSession, say } = require('./helpers');

const shots = process.env.AGENT_WEB_SHOTS;

test.describe('wide screen: four columns', () => {
  test.use({ viewport: { width: 1440, height: 900 } });

  test('columns run left to right: notifications, sessions, chat, tools', async ({ page }) => {
    await login(page);
    const xs = [];
    for (const sel of ['.notes', '.tree', '.chat', '.tools']) {
      await expect(page.locator(sel)).toBeVisible();
      xs.push((await page.locator(sel).boundingBox()).x);
    }
    expect([...xs].sort((a, b) => a - b)).toEqual(xs);
  });

  test('bar: quotas and metrics on top, tools are icons with screen reader labels', async ({ page }) => {
    await login(page);
    const bar = await page.locator('.bar').boundingBox();
    expect(bar.y).toBeLessThan(40);
    const tools = page.getByRole('group', { name: 'Tool windows' });
    for (const name of ['Terminal', 'Search', 'Schedule', 'Log', 'Settings']) {
      const b = tools.getByRole('button', { name, exact: true });
      await expect(b).toBeVisible();
      expect((await b.innerText()).trim()).toBe('');
    }
  });

  test('chat: the "Write to the agent…" field and a round send button', async ({ page }) => {
    await login(page);
    await expect(page.getByLabel('Message', { exact: true })).toHaveAttribute('placeholder', 'Write to the agent…');
    const send = page.getByRole('button', { name: 'Send' });
    const box = await send.boundingBox();
    expect(Math.abs(box.width - box.height)).toBeLessThan(1);
    expect(await send.evaluate((e) => getComputedStyle(e).borderRadius)).toMatch(/50%|999|20px/);
    if (shots) await page.screenshot({ path: `${shots}/wide-1440.png` });
  });

  test('sessions: "New session" and "New folder" are rows with an icon', async ({ page }) => {
    await login(page);
    for (const name of ['New session', 'New folder']) {
      const b = page.getByRole('button', { name, exact: true });
      await expect(b).toBeVisible();
      await expect(b.locator('svg')).toHaveCount(1);
    }
  });
});

test.describe('narrow screen', () => {
  test.use({ viewport: { width: 375, height: 812 } });

  test('no horizontal scroll, tools are reachable as an icon strip', async ({ page }) => {
    await login(page);
    expect(await page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth)).toBeLessThanOrEqual(0);
    await expect(page.getByRole('button', { name: 'Settings', exact: true })).toBeVisible();
    if (shots) await page.screenshot({ path: `${shots}/narrow-375.png` });
  });
});

// Dark theme shots: with an open session and messages (needs an agent reply from the core).
for (const [name, viewport] of [['wide-1440', { width: 1440, height: 900 }], ['narrow-375', { width: 375, height: 812 }]]) {
  test.describe(`dark theme ${name}`, () => {
    test.use({ viewport, colorScheme: 'dark' });
    test('session with messages', async ({ page }) => {
      await login(page);
      await newSession(page);
      await say(page, 'Hello, visual check');
      await expect(page.locator('article.bubble.mine')).toBeVisible();
      await expect(page.locator('article.bubble.agent').first()).toBeVisible({ timeout: 15_000 });
      if (viewport.width > 720) {
        expect(await page.evaluate(() => getComputedStyle(document.body).backgroundImage)).toContain('radial-gradient');
      }
      if (shots) await page.screenshot({ path: `${shots}/dark-${name}.png` });
    });
  });
}
