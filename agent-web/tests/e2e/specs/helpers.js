const { expect } = require('@playwright/test');

function token(name = 'AGENT_WEB_TOKEN') {
  const t = process.env[name];
  if (!t) throw new Error(`variable ${name} is not set (the token is issued on the VM and never stored in the repository)`);
  return t;
}

async function login(page, tok = token()) {
  await page.goto('/', { waitUntil: 'domcontentloaded' });
  await page.locator('#tok').fill(tok);
  await page.getByRole('button', { name: 'Sign in' }).click();
  await expect(page.getByRole('status').filter({ hasText: 'Connected' })).toBeVisible({ timeout: 15_000 });
}

async function newSession(page) {
  // A narrow screen shows one section: go to Sessions first.
  const nav = page.getByRole('navigation', { name: 'Sections' });
  if (await nav.isVisible()) await nav.getByRole('button', { name: 'Sessions' }).click();
  // The tree arrives as a snapshot after sign-in: wait until the row count stops changing.
  let before = -1;
  for (let i = 0; i < 20; i++) {
    const n = await page.locator('[role=treeitem]').count();
    if (n === before) break;
    before = n;
    await page.waitForTimeout(400);
  }
  await page.getByRole('button', { name: 'New session', exact: true }).click();
  await expect(page.locator('[role=treeitem]')).toHaveCount(before + 1, { timeout: 15_000 });
  await expect(page.getByLabel('Message', { exact: true })).toBeEnabled({ timeout: 15_000 });
}

async function say(page, text) {
  const box = page.getByLabel('Message', { exact: true });
  await box.fill(text);
  await box.press('Enter');
}

const { execFileSync } = require('child_process');
const path = require('path');

// Issue and revoke tokens on the dev VM (revocation scenario only; the token value never reaches the logs).
function issueTempToken(name) {
  const script = path.join(process.env.AGENT_WEB_SCRIPTS || '', 'dev-vm-token.sh');
  return execFileSync('bash', [script, name, 'chat', '1'], { encoding: 'utf8' }).trim().split('\n').pop();
}

function revokeByName(name) {
  const host = process.env.DEV_VM_HOST;
  if (!host) throw new Error('set DEV_VM_HOST to an ssh host');
  const dest = process.env.DEV_VM_DEST || 'agent-web-stand';
  const bin = `$HOME/${dest}/target/release/agentd`;
  const sock = '"/run/user/$(id -u)/agentd.sock"';
  const list = execFileSync('ssh', [host, `${bin} token list --socket ${sock}`], { encoding: 'utf8' });
  const row = list.split('\n').filter((l) => l.split('\t')[1] === name && l.endsWith('active')).pop();
  if (!row) throw new Error('token to revoke not found');
  execFileSync('ssh', [host, `${bin} token revoke --id ${row.split('\t')[0]} --socket ${sock}`]);
}

module.exports = { token, login, newSession, say, issueTempToken, revokeByName };
