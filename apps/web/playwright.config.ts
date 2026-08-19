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
        // Empty -> instrumentation.ts bails out cleanly. Setting it
        // to anything (including the literal "true" string) keeps
        // Next.js itself happy; the OTel SDK is gated on the
        // presence of OTEL_EXPORTER_OTLP_ENDPOINT, so leaving this
        // empty disables tracing without crashing the boot path.
        OTEL_EXPORTER_OTLP_ENDPOINT: '',
        // Plain JSON logs — pino-pretty's worker thread can hang
        // when stdout is piped under webServer.
        LOG_LEVEL: 'warn',
      },
    },
  ],
});
