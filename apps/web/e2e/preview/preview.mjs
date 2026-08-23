/**
 * Local preview of the projection port — a browsable instance, signed in,
 * against the e2e mock API.
 *
 *   pnpm --filter web run preview
 *
 * It boots the same two processes `playwright.config.ts` boots (the mock API on
 * 3199, `next dev` on 3000 pointed at it), loads the default fixture scenario,
 * and opens a Chromium window whose session cookie is already set.
 *
 * The cookie has to be set for you: `starstats_session` is httpOnly, so it
 * cannot be written from the browser console, and there is no dev-login route
 * in the app (adding one would be a product change to serve a preview). That is
 * the whole reason this script drives a browser rather than just printing a
 * URL.
 *
 * NOTHING HERE IS REAL DATA. Every response is an e2e fixture, so figures are
 * whatever the fixture says. It shows layout, chrome and interaction — not your
 * account.
 */
import { spawn } from 'node:child_process';
import { chromium } from '@playwright/test';

const MOCK_PORT = 3199;
const WEB_PORT = 3000;
const MOCK_BASE = `http://localhost:${MOCK_PORT}`;
const WEB_BASE = `http://localhost:${WEB_PORT}`;

const HANDLE = process.env.PREVIEW_HANDLE ?? 'TestPilot';
// Staff grants are opt-in so the admin console is not in the nav by default.
// `PREVIEW_ROLES=admin` to see it.
const ROLES = process.env.PREVIEW_ROLES
  ? process.env.PREVIEW_ROLES.split(',')
  : [];
const START = process.env.PREVIEW_PATH ?? '/me';

const children = [];
function boot(label, command, args, env) {
  const child = spawn(command, args, {
    env: { ...process.env, ...env },
    shell: process.platform === 'win32',
    stdio: ['ignore', 'pipe', 'pipe'],
  });
  const tag = (buf) =>
    String(buf)
      .split('\n')
      .filter((l) => l.trim())
      .forEach((l) => console.log(`[${label}] ${l}`));
  child.stdout.on('data', tag);
  child.stderr.on('data', tag);
  children.push(child);
  return child;
}

/** Bounded wait — never poll forever, per the repo's shell rules. */
async function waitFor(url, label, timeoutMs = 180_000) {
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    try {
      const r = await fetch(url);
      if (r.ok || r.status === 404) return;
    } catch {
      // not up yet
    }
    if (Date.now() > deadline) throw new Error(`${label} did not start in time`);
    await new Promise((r) => setTimeout(r, 500));
  }
}

function shutdown(code = 0) {
  for (const c of children) {
    try {
      c.kill();
    } catch {
      // already gone
    }
  }
  process.exit(code);
}
// Both signals, and `exit` as the backstop. Node does not kill spawned children
// for you, so without this a Ctrl-C leaves `next dev` and the mock API holding
// ports 3000 and 3199 — which then look "already running" to the next boot and
// silently serve stale code.
process.on('SIGINT', () => shutdown(0));
process.on('SIGTERM', () => shutdown(0));
process.on('exit', () => {
  for (const c of children) {
    try {
      c.kill();
    } catch {
      // already gone
    }
  }
});

boot('mock', 'node', ['e2e/mock-server/server.mjs'], {
  MOCK_PORT: String(MOCK_PORT),
});
await waitFor(`${MOCK_BASE}/__mock/health`, 'mock API', 30_000);

boot('web', 'npx', ['next', 'dev', '-p', String(WEB_PORT)], {
  STARSTATS_API_URL: MOCK_BASE,
  // Same reason as the Playwright config: the KB reference fetchers otherwise
  // cache the first scenario's response for an hour.
  STARSTATS_DISABLE_FETCH_CACHE: '1',
  OTEL_EXPORTER_OTLP_ENDPOINT: '',
  LOG_LEVEL: 'warn',
});
await waitFor(WEB_BASE, 'next dev');

// The same default fixture map the capture specs run against, so the preview
// shows the data the screenshots were taken against. Imported straight from the
// TypeScript helper — Node strips the types itself (>= 22.6; this repo is on
// 26), so there is no build step and no extra dependency for a dev tool. It is
// deliberately NOT wrapped in a catch: the mock server's default scenario is
// empty, so a silent failure here would serve `599 no_mock_fixture` on every
// endpoint and the preview would look broken for the wrong reason.
const { scenarioFor } = await import('../helpers/api-mock.ts');
const scenarioResp = await fetch(`${MOCK_BASE}/__mock/scenario`, {
  method: 'POST',
  headers: { 'content-type': 'application/json' },
  body: JSON.stringify(scenarioFor('preview')),
});
if (!scenarioResp.ok) {
  console.error(`Loading the fixture scenario failed: ${scenarioResp.status}`);
  shutdown(1);
}

const browser = await chromium.launch({ headless: false });
const context = await browser.newContext({ viewport: { width: 1440, height: 900 } });
await context.addCookies([
  {
    name: 'starstats_session',
    value: JSON.stringify({
      t: 'test-token',
      u: 'user_test',
      h: HANDLE,
      v: true,
      r: ROLES,
    }),
    domain: 'localhost',
    path: '/',
    httpOnly: true,
    sameSite: 'Lax',
  },
]);

const page = await context.newPage();
await page.goto(`${WEB_BASE}${START}`);

console.log('');
console.log(`  Preview ready   ${WEB_BASE}${START}`);
console.log(`  Signed in as    @${HANDLE}${ROLES.length ? ` (${ROLES.join(', ')})` : ''}`);
console.log('  Ported surfaces /me  /settings  /sharing  /devices  /me/travel');
console.log('                  /me/contracts  /me/loadout  /kb  /kb/vehicle');
console.log('                  /kb/vehicle/<slug>');
console.log('  Data is e2e fixtures, not your account. Ctrl-C to stop.');
console.log('');

browser.on('disconnected', () => shutdown(0));
