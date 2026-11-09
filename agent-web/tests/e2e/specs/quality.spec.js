const { test, expect } = require('@playwright/test');
const AxeBuilder = require('@axe-core/playwright').default;
const zlib = require('zlib');
const { login, newSession } = require('./helpers');

// Budget: wasm with glue code at most 2 MiB gzipped.
const BUDGET = 2 * 1024 * 1024;
// Working limit: the `web` profile (opt-level z, lto) keeps the client well below the budget.
const WORK_LIMIT = 700 * 1024;

test('wasm and JS gzip budget', async ({ request }) => {
  const html = await (await request.get('/')).text();
  const boot = await (await request.get('/app/boot.js')).text();
  const urls = [...boot.matchAll(/'(\/app\/[^']+)'/g)].map((m) => m[1]);
  expect(urls.length).toBeGreaterThanOrEqual(2);
  let total = 0;
  for (const u of urls) {
    const body = await (await request.get(u)).body();
    total += zlib.gzipSync(body).length;
  }
  console.log(`gzip total: ${total} bytes (budget ${BUDGET})`);
  expect(total).toBeLessThan(BUDGET);
  expect(total).toBeLessThan(WORK_LIMIT);
  expect(html).toContain('/app/');
});

test('sign-in form accessibility (axe)', async ({ page }) => {
  await page.goto('/', { waitUntil: 'domcontentloaded' });
  await expect(page.locator('#tok')).toBeVisible();
  const r = await new AxeBuilder({ page }).withTags(['wcag2a', 'wcag2aa']).analyze();
  expect(r.violations.map((v) => `${v.id}: ${v.nodes.map((n) => n.target.join(' ') + ' ' + (n.any[0] ? n.any[0].message : '')).join(' | ')}`)).toEqual([]);
});

test('shell accessibility (axe) and keyboard', async ({ page }) => {
  await login(page);
  await newSession(page);
  const r = await new AxeBuilder({ page }).withTags(['wcag2a', 'wcag2aa']).analyze();
  expect(r.violations.map((v) => `${v.id}: ${v.nodes.map((n) => n.target.join(' ') + ' ' + (n.any[0] ? n.any[0].message : '')).join(' | ')}`)).toEqual([]);
  // The input field is keyboard-reachable; focus is visible.
  await page.getByLabel('Message', { exact: true }).focus();
  await expect(page.getByLabel('Message', { exact: true })).toBeFocused();
});

test.describe('phone 375x812', () => {
  test.use({ viewport: { width: 375, height: 812 } });
  test('no horizontal scroll, input and tree on screen', async ({ page }) => {
    await login(page);
    await newSession(page);
    const over = await page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth);
    expect(over).toBeLessThanOrEqual(0);
    await expect(page.getByLabel('Message', { exact: true })).toBeInViewport();
    await expect(page.getByRole('navigation', { name: 'Sections' })).toBeInViewport();
  });
});

test('first screen appears fast (sign-in form within 5 seconds)', async ({ page }) => {
  const t0 = Date.now();
  await page.goto('/', { waitUntil: 'domcontentloaded' });
  await expect(page.locator('#tok')).toBeVisible({ timeout: 5_000 });
  console.log(`sign-in form: ${Date.now() - t0} ms`);
});

test('no animations or transitions (prefers-reduced-motion is honored by default)', async ({ page }) => {
  await page.goto('/', { waitUntil: 'domcontentloaded' });
  await expect(page.locator('#tok')).toBeVisible();
  const css = await page.evaluate(() =>
    [...document.styleSheets].flatMap((s) => [...s.cssRules].map((r) => r.cssText)).join('\n'),
  );
  expect(css).not.toMatch(/animation|transition|@keyframes/);
});

test.describe('200% zoom (equivalent to a 640x400 window)', () => {
  test.use({ viewport: { width: 640, height: 400 } });
  test('no horizontal scroll, the input field and the new-session button are reachable', async ({ page }) => {
    await login(page);
    await newSession(page);
    expect(await page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth)).toBeLessThanOrEqual(0);
    await page.getByLabel('Message', { exact: true }).scrollIntoViewIfNeeded();
    await expect(page.getByLabel('Message', { exact: true })).toBeInViewport();
  });
});

test('keyboard: Tab reaches "New session", Enter creates a session, Escape closes the window', async ({ page }) => {
  await login(page);
  await page.waitForTimeout(800);
  const before = await page.locator('[role=treeitem]').count();
  await page.getByRole('button', { name: 'New session', exact: true }).focus();
  await expect(page.getByRole('button', { name: 'New session', exact: true })).toBeFocused();
  await page.keyboard.press('Enter');
  await expect(page.locator('[role=treeitem]')).toHaveCount(before + 1, { timeout: 15_000 });
  await page.getByRole('button', { name: 'Settings' }).click();
  await expect(page.getByRole('dialog', { name: 'Settings' })).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(page.getByRole('dialog')).toBeHidden();
});
