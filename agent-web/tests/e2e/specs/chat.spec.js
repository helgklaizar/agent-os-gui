const { test, expect } = require('@playwright/test');
const { login, newSession, say } = require('./helpers');

test('sign-in, session, streamed reply, markup and literal HTML', async ({ page }) => {
  await login(page);
  await newSession(page);
  await say(page, 'Hello <b>html</b> and **bold** `code`');
  const flow = page.locator('.flow');
  await expect(flow.locator('article.bubble.agent').first()).toContainText('Reply:', { timeout: 20_000 });
  await expect(flow.locator('article.mine strong')).toHaveText('bold');
  await expect(flow.locator('article.mine code')).toHaveText('code');
  await expect(flow.locator('article.mine')).toContainText('<b>html</b>');
  expect(await flow.locator('b').count()).toBe(0);
});

test('wrong token: a clear reason, the form stays', async ({ page }) => {
  await page.goto('/', { waitUntil: 'domcontentloaded' });
  await page.locator('#tok').fill('agt_wrongtoken');
  await page.getByRole('button', { name: 'Sign in' }).click();
  await expect(page.getByRole('alert')).toContainText('Token not accepted', { timeout: 15_000 });
  await expect(page.locator('#tok')).toBeVisible();
});

test('a token with non-ASCII characters is rejected at once, without calling the core', async ({ page }) => {
  await page.goto('/', { waitUntil: 'domcontentloaded' });
  await page.locator('#tok').fill('agt_inv\u00e9lid');
  await page.getByRole('button', { name: 'Sign in' }).click();
  await expect(page.getByRole('alert')).toContainText('Token not accepted', { timeout: 15_000 });
});

test('connection drop: the message is queued and delivered after the connection returns', async ({ page, context }) => {
  await login(page);
  await newSession(page);
  await context.setOffline(true);
  await say(page, 'while offline');
  await expect(page.getByText('Queued: 1')).toBeVisible({ timeout: 15_000 });
  await context.setOffline(false);
  await expect(page.getByText('Queued:')).toBeHidden({ timeout: 30_000 });
  await expect(page.locator('article.bubble.mine', { hasText: 'while offline' })).not.toContainText('sending');
  await expect(page.locator('article.bubble.agent').last()).toContainText('Reply:', { timeout: 20_000 });
});

test('permission request: a card, a one-off decision, no "Always"', async ({ page }) => {
  test.skip(!process.env.AGENT_WEB_TOKEN_DECIDE, 'needs a web token with the decide right (AGENT_WEB_TOKEN_DECIDE)');
  await login(page, process.env.AGENT_WEB_TOKEN_DECIDE);
  await newSession(page);
  await say(page, '#scn:action_send write a letter');
  const card = page.locator('.card.ask');
  await expect(card).toBeVisible({ timeout: 20_000 });
  await expect(card.getByRole('button', { name: 'Allow' })).toBeVisible();
  await expect(card.getByRole('button', { name: /Always/ })).toHaveCount(0);
  await card.getByRole('button', { name: 'Allow' }).click();
  await expect(card).toContainText('allowed', { timeout: 15_000 });
});
