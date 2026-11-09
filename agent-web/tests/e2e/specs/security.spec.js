const { test, expect } = require('@playwright/test');
const { login, newSession, say } = require('./helpers');

// A set of "evil strings" (counterpart of controllers::testkit::evil_strings): no new element may appear in the DOM
// from message text and no code may run.
const EVIL = [
  '<img src=x onerror="window.__xss=1">',
  '<script>window.__xss=1</script>',
  '"><svg onload="window.__xss=1">',
  '<iframe srcdoc="<script>parent.__xss=1</script>"></iframe>',
  '[click](javascript:window.__xss=1)',
  '[click](JaVa\tScRiPt:window.__xss=1)',
  '[data](data:text/html,<script>window.__xss=1</script>)',
  '<a href="javascript:window.__xss=1">x</a>',
  '<style>body{display:none}</style>',
  '<meta http-equiv="refresh" content="0;url=http://evil.example">',
  '‮right-to-left text‬ and <b>tag</b>',
  '{{7*7}} ${7*7} <%= 7*7 %>',
  '`<img src=x onerror=window.__xss=1>`',
  '```\n<script>window.__xss=1</script>\n```',
];

test('evil strings create no elements and do not run', async ({ page }) => {
  await login(page);
  await newSession(page);
  const flow = page.locator('.flow');
  for (const s of EVIL) {
    const before = await flow.locator('article.bubble').count();
    await say(page, s);
    // Own message plus the agent reply (`Reply: …` with the same text).
    await expect(flow.locator('article.bubble')).toHaveCount(before + 2, { timeout: 25_000 });
  }
  await page.waitForTimeout(500);
  expect(await page.evaluate(() => window.__xss)).toBeUndefined();
  for (const tag of ['img', 'script', 'iframe', 'svg', 'style', 'meta', 'b']) {
    expect(await flow.locator(tag).count(), tag).toBe(0);
  }
  // Links: only http, https, mailto.
  const hrefs = await flow.locator('a').evaluateAll((as) => as.map((a) => a.getAttribute('href')));
  for (const h of hrefs) expect(h).toMatch(/^(https?:|mailto:)/);
  // The literal text is visible.
  await expect(flow).toContainText('<img src=x onerror="window.__xss=1">');
  await expect(page).toHaveTitle('Agent OS');
});

test('the page is served with CSP and without inline scripts', async ({ request }) => {
  const r = await request.get('/');
  const csp = r.headers()['content-security-policy'];
  expect(csp).toContain("script-src 'self' 'wasm-unsafe-eval'");
  expect(csp).not.toContain('unsafe-inline');
  expect(r.headers()['x-content-type-options']).toBe('nosniff');
  const html = await r.text();
  expect(html).not.toMatch(/<script(?![^>]*\bsrc=)/);
});

test('without a token the data is closed', async ({ request }) => {
  expect((await request.get('/v1/sessions')).status()).toBe(401);
});
