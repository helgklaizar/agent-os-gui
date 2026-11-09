const { test, expect } = require('@playwright/test');
const { login, newSession, say, issueTempToken, revokeByName } = require('./helpers');

test('token revoked on an open tab: close 4401, sign-in form with a reason, token forgotten', async ({ page }) => {
  test.skip(!process.env.AGENT_WEB_REVOKE, 'needs ssh access to the dev VM (AGENT_WEB_REVOKE=1)');
  const name = `e2e-revoke-${Date.now()}`;
  const tok = issueTempToken(name);
  await login(page, tok);
  await newSession(page);
  await say(page, 'before revoke');
  await expect(page.locator('article.bubble.agent').last()).toContainText('Reply:', { timeout: 20_000 });
  const closes = [];
  page.on('websocket', (ws) => ws.on('close', () => closes.push('close')));
  revokeByName(name);
  // The core closes the WS with code 4401: the client does not reconnect but shows the sign-in form with a reason.
  await expect(page.getByRole('alert')).toContainText('Token revoked', { timeout: 20_000 });
  await expect(page.locator('#tok')).toBeVisible();
  expect(await page.evaluate(() => sessionStorage.getItem('agentd.token'))).toBeNull();
  // Signing in again with the same token is rejected.
  await page.locator('#tok').fill(tok);
  await page.getByRole('button', { name: 'Sign in' }).click();
  await expect(page.getByRole('alert')).toContainText('Token not accepted', { timeout: 15_000 });
});
