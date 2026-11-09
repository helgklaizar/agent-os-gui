const { test, expect } = require('@playwright/test');
const AxeBuilder = require('@axe-core/playwright').default;
const { login, newSession, say } = require('./helpers');

test('schedule: the window opens, a list or an empty state, accessibility', async ({ page }) => {
  await login(page);
  await page.getByRole('button', { name: 'Schedule' }).click();
  const dlg = page.getByRole('dialog', { name: 'Schedule' });
  await expect(dlg).toBeVisible();
  await expect(dlg.locator('.hint').first()).not.toHaveText('Loading…', { timeout: 15_000 });
  const r = await new AxeBuilder({ page }).include('[role=dialog]').withTags(['wcag2a', 'wcag2aa']).analyze();
  expect(r.violations.map((v) => v.id)).toEqual([]);
  await dlg.getByRole('button', { name: 'Close' }).click();
  await expect(dlg).toBeHidden();
});

test('search: Ctrl+K opens the window, the query goes to /search', async ({ page }) => {
  await login(page);
  await newSession(page);
  const marker = `marker${Date.now()}`;
  await say(page, `find ${marker}`);
  await expect(page.locator('article.bubble.agent').last()).toContainText(marker, { timeout: 20_000 });
  await page.keyboard.press('Control+k');
  const dlg = page.getByRole('dialog', { name: 'Search' });
  await expect(dlg).toBeVisible();
  const req = page.waitForRequest((r) => r.url().includes('/v1/search?q='));
  await dlg.getByLabel('Query').fill(marker);
  await dlg.getByRole('button', { name: 'Find' }).click();
  expect(decodeURIComponent((await req).url())).toContain(marker);
  // Outcome: results or an honest message "Nothing found" / "Search unavailable"; a stuck "Searching…" must not remain.
  await expect(dlg.locator('.hint').last()).not.toHaveText('Searching…', { timeout: 15_000 });
  await page.keyboard.press('Escape');
});

test('agent terminal: a view-only window', async ({ page }) => {
  await login(page);
  await page.getByRole('button', { name: 'Terminal' }).click();
  const dlg = page.getByRole('dialog', { name: 'Agent terminal' });
  await expect(dlg).toBeVisible();
  await expect(dlg.getByText('View only')).toBeVisible();
  await expect(dlg.locator('input, textarea')).toHaveCount(0);
});

test.describe('dark theme', () => {
  test.use({ colorScheme: 'dark' });
  test('contrast of the shell and the search window (axe)', async ({ page }) => {
    await login(page);
    await newSession(page);
    await page.getByRole('button', { name: 'Search' }).click();
    const r = await new AxeBuilder({ page }).withTags(['wcag2a', 'wcag2aa']).analyze();
    expect(r.violations.map((v) => `${v.id}: ${v.nodes.map((n) => n.target.join(' ')).join(' | ')}`)).toEqual([]);
  });
});
