import { defineConfig, devices } from '@playwright/test';

const WEB_PORT = 3100;
const MOCK_PORT = 3199;

/**
 * Playwright config for the StarStats web app.
 *
 * Two web servers are started:
 *   - the mock API on `MOCK_PORT` — every test injects its own
 *     scenario before navigating;
 *   - the Next dev server on `WEB_PORT` with `STARSTATS_API_URL`
 *     pointed at the mock.
 *
 * The dev port is deliberately 3100 (not 3000) so a developer can
 * run `pnpm dev` for manual work without colliding with the test
 * server. `reuseExistingServer` is honored locally; CI starts fresh
 * each time.
 *
 * Workers are forced to 1 — the mock server keeps a single active
 * scenario in memory, so parallel tests would race.
 */
export default defineConfig({
  testDir: './e2e',
  testIgnore: ['**/mock-server/**'],
  timeout: 30_000,
  expect: { timeout: 5_000 },
  fullyParallel: false,
  workers: 1,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 1 : 0,
  reporter: process.env.CI ? 'github' : 'list',
  use: {
    baseURL: `http://localhost:${WEB_PORT}`,
    trace: 'retain-on-failure',
    screenshot: 'only-on-failure',
    video: 'retain-on-failure',
    actionTimeout: 5_000,
    navigationTimeout: 10_000,
  },
  projects: [
    {
      name: 'chromium',
      use: { ...devices['Desktop Chrome'] },
    },
  ],
  webServer: [
    {
      // Mock API — boots first because Next dev fetches it on warm-up.
      command: `node e2e/mock-server/server.mjs`,
      port: MOCK_PORT,
      reuseExistingServer: !process.env.CI,
      stdout: 'pipe',
      stderr: 'pipe',
      env: {
        MOCK_PORT: String(MOCK_PORT),
      },
    },
    {
      // OTel SDK packages are externalized via
      // `serverExternalPackages` in `next.config.mjs`, so the default
      // webpack-based dev bundler no longer 500s on the dynamic
      // imports inside `instrumentation.ts`. Tracing itself stays
      // disabled below by leaving `OTEL_EXPORTER_OTLP_ENDPOINT` empty.
      command: `next dev -p ${WEB_PORT}`,
      port: WEB_PORT,
      reuseExistingServer: !process.env.CI,
      timeout: 180_000,
      stdout: 'pipe',
      stderr: 'pipe',
      env: {
        STARSTATS_API_URL: `http://localhost:${MOCK_PORT}`,
        // Disable Next's fetch cache for KB reference endpoints —
        // tests reset the mock per-scenario but Next would otherwise
        // serve a cached response from a prior scenario across
        // `page.goto()` calls. Read by `kbCacheOpts()` in
        // `src/lib/reference.ts`. Production keeps the 1h revalidate.
        STARSTATS_DISABLE_FETCH_CACHE: '1',
        // `/downloads` (the Emitter) reads the tray release feed from GitHub.
        // It absorbed `/devices`, so the auth and pairing specs land on it —
        // point the feed at the mock server so no test makes a real,
        // rate-limited call to api.github.com. Fixture: `GET /gh/releases`
        // in `scenarioFor`'s base map.
        STARSTATS_RELEASES_API: `http://localhost:${MOCK_PORT}/gh/releases`,
        // Empty -> instrumentation.ts bails out cleanly. Setting it
        // to anything (including the literal "true" string) keeps
        // Next.js itself happy; the OTel SDK is gated on the
        // presence of OTEL_EXPORTER_OTLP_ENDPOINT, so leaving this
        // empty disables tracing without crashing the boot path.
        OTEL_EXPORTER_OTLP_ENDPOINT: '',
        // Plain JSON logs — pino-pretty's worker thread can hang
        // when stdout is piped under webServer.
        LOG_LEVEL: 'warn',
        // NOTE ON MEMORY — deliberately no `--max-old-space-size` here.
        //
        // This server accumulates compiled modules for the whole run
        // and never gives them back: measured 2026-09-16 at ~25 MB per
        // test, climbing monotonically to a 9.1 GB peak across 362
        // tests in one unsharded run. On that date CI hit `FATAL
        // ERROR: Ineffective mark-compacts near heap limit -
        // JavaScript heap out of memory` 26 minutes into a 30.8-minute
        // run; the 92 tests after it failed `ERR_CONNECTION_REFUSED`,
        // which reads as 92 broken tests and was one dead server.
        //
        // The fix is SHARDING, not a bigger heap — see ci.yml's
        // web-e2e job, where four sequential shards each start a fresh
        // server; shard 1/4 measured a 4.0 GB peak over its 91 tests.
        // That job pins `--max-old-space-size` in its own step env,
        // where sharding guarantees the ceiling cannot bind.
        //
        // Nothing is pinned HERE because a ceiling low enough to
        // protect CI would kill an unsharded local run, which
        // legitimately needs ~9 GB. Node's own default, which scales
        // with host memory, is the better behaviour for a developer
        // machine.
        //
        // So: running the whole suite locally, expect ~9 GB.
        // `playwright test --shard=1/4` keeps it near 4 GB.
      },
    },
  ],
});
