// Agent terminal with live output (terminal.* frames from agentd) and the 429 limit.
// Needs a second agentd instance with an agent runner: AGENT_WEB_URL_TERM, AGENT_WEB_TOKEN_TERM (a web decide token),
// AGENT_WEB_PROJECT_TERM (project directory allowed to the runner). Script: agent-web/scripts/dev-vm-term-stand.sh.
const { test, expect } = require('@playwright/test');
const { login, newSession, say } = require('./helpers');

const URL = process.env.AGENT_WEB_URL_TERM;
const TOKEN = process.env.AGENT_WEB_TOKEN_TERM;
const PROJECT = process.env.AGENT_WEB_PROJECT_TERM;

test.describe('terminal and limit on a separate instance', () => {
  test.skip(!URL || !TOKEN, 'needs an instance with a runner (AGENT_WEB_URL_TERM, AGENT_WEB_TOKEN_TERM)');
  test.use({ baseURL: URL });

  test('live output: command, stdout chunks, status, ANSI color, text without markup', async ({ page, request }) => {
    // A session with a project directory is created via the API: the UI creates sessions without one.
    const headers = { Authorization: `Bearer ${TOKEN}` };
    const personas = await (await request.get('/v1/personas', { headers })).json();
    const r = await request.post('/v1/sessions', {
      headers,
      data: { name: 'terminal e2e', path: PROJECT, persona_id: personas[0].id },
    });
    expect(r.status()).toBe(201);
    await login(page, TOKEN);
    await page.getByRole('treeitem', { name: /terminal e2e/ }).first().click();
    await expect(page.getByLabel('Message', { exact: true })).toBeEnabled({ timeout: 15_000 });
    await say(page, "#scn:action_shellrun printf 'hello <b>x</b>\\n\\033[31mred\\033[0m\\n'; echo error 1>&2");
    // The `shell` right on the stand may be `allow` (the command runs at once) or `ask` (a one-off web token decision is needed).
    const card = page.locator('.card.ask');
    await card.waitFor({ timeout: 6_000 }).then(
      () => card.getByRole('button', { name: 'Allow' }).click(),
      () => {},
    );
    await page.getByRole('button', { name: 'Terminal', exact: true }).click();
    const dlg = page.getByRole('dialog', { name: 'Agent terminal' });
    const term = dlg.locator('pre.term');
    await expect(term).toContainText('$ printf', { timeout: 20_000 });
    await expect(term).toContainText('hello <b>x</b>');
    await expect(term).toContainText('red');
    await expect(term).toContainText('error');
    await expect(term).toContainText('[exited (code 0)]');
    expect(await term.locator('b').count()).toBe(0);
    await expect(term.locator('.fg1', { hasText: 'red' })).toHaveCount(1);
    await expect(term.locator('.err', { hasText: 'error' })).toHaveCount(1);
    await expect(dlg.getByText('No output yet')).toBeHidden();
    await expect(dlg.locator('input, textarea')).toHaveCount(0);
  });

  test('failed attempt limit: 429 with Retry-After, the sign-in form names the reason', async ({ page, request }) => {
    test.setTimeout(150_000);
    for (let i = 0; i < 5; i++) {
      const r = await request.get('/v1/folders', { headers: { Authorization: 'Bearer agt_wrong' } });
      expect(r.status()).toBe(401);
    }
    const blocked = await request.get('/v1/folders', { headers: { Authorization: 'Bearer agt_wrong' } });
    expect(blocked.status()).toBe(429);
    const wait = Number(blocked.headers()['retry-after']);
    expect(wait).toBeGreaterThanOrEqual(1);
    expect(wait).toBeLessThanOrEqual(60);
    expect((await blocked.json()).error.code).toBe('rate_limited');
    await page.goto('/', { waitUntil: 'domcontentloaded' });
    await page.locator('#tok').fill(TOKEN);
    await page.getByRole('button', { name: 'Sign in' }).click();
    await expect(page.getByRole('alert')).toContainText('Too many requests', { timeout: 15_000 });
    // The pause ends: the same token is accepted afterwards.
    await page.waitForTimeout((wait + 2) * 1000);
    await login(page, TOKEN);
  });
});
