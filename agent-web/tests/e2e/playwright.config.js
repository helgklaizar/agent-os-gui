// Browser scenarios of agent-web against a running agentd. The address and tokens come from the environment and are never logged.
// Browsers: AGENT_WEB_BROWSERS=chromium,firefox,webkit (all three by default).
const { defineConfig, devices } = require('@playwright/test');

const all = {
  chromium: devices['Desktop Chrome'],
  firefox: devices['Desktop Firefox'],
  webkit: devices['Desktop Safari'],
};
const wanted = (process.env.AGENT_WEB_BROWSERS || 'chromium,firefox,webkit').split(',').map((s) => s.trim());

module.exports = defineConfig({
  testDir: './specs',
  timeout: 90_000,
  workers: 1,
  reporter: [['list']],
  use: {
    baseURL: process.env.AGENT_WEB_URL || 'http://127.0.0.1:4096',
    headless: true,
    viewport: { width: 1280, height: 800 },
  },
  // Firefox on the VM occasionally fails to fire the load event (a browser glitch, not the client's): one retry, flakiness shows in the report.
  projects: wanted.map((name) => ({
    name,
    retries: name === 'firefox' ? 1 : 0,
    use: { ...all[name], viewport: { width: 1280, height: 800 } },
  })),
});
