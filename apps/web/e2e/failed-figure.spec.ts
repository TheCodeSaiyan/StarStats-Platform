import { test, expect } from '@playwright/test';
import {
  loginAs,
  resetScenario,
  scenarioFor,
  setScenario,
} from './helpers/api-mock';
import { liveIn } from './helpers/shell';

/**
 * A failed fetch must never be formatted as a number.
 *
 * Reported as "when Logged flight time goes above 1024, it resets to 0".
 * Nothing was wiped: `session_summary` held 2,289 sessions and 2,359 hours
 * while the page rendered 0h. The chain was
 *
 *   sessions_dirty stuck true (set by ingest, cleared by a rebuild, so an
 *   actively-syncing account is perpetually dirty)
 *     -> every read rebuilds the rollup: DELETE + a window-function
 *        re-INSERT over the entire event history
 *     -> past some history size that exceeds its timeout
 *     -> /v1/me/stats/playtime correctly returns 500
 *     -> settledOr correctly resolves it to null
 *     -> `playtime?.total_playtime_secs ?? 0` INVENTED a zero
 *     -> formatPlaytime(0) === "0h"
 *
 * The threshold is why it looked like a magic number and why it came back on
 * its own. The zero is why it looked like data loss.
 */
test.describe('a failed figure does not read as zero', () => {
  test.beforeEach(async ({ request }) => {
    await resetScenario(request);
  });

  test('renders a dash, not 0h, when playtime fails', async ({
    page,
    request,
  }) => {
    await setScenario(
      request,
      scenarioFor('failed-playtime', {
        'GET /v1/me/stats/playtime': { status: 500, body: { error: 'query_failed' } },
      }),
    );
    await loginAs(page, { handle: 'TestPilot' });
    await page.setViewportSize({ width: 1600, height: 950 });
    await page.goto('/me', { waitUntil: 'domcontentloaded', timeout: 40_000 });

    const core = liveIn(page, '.hp-core');
    await expect(core).toBeVisible({ timeout: 20_000 });
    await expect(
      core,
      'a broken query must not be formatted as a real figure',
    ).not.toContainText('0h');
    await expect(core).toContainText('—');
  });

  test('still shows 0h for an account that genuinely has none', async ({
    page,
    request,
  }) => {
    // The other half of the contract. Suppressing a REAL zero would be the
    // same fault in reverse — an empty account has an answer, and it is 0.
    await setScenario(
      request,
      scenarioFor('zero-playtime', {
        'GET /v1/me/stats/playtime': {
          status: 200,
          body: { total_playtime_secs: 0, session_count: 0 },
        },
      }),
    );
    await loginAs(page, { handle: 'TestPilot' });
    await page.setViewportSize({ width: 1600, height: 950 });
    await page.goto('/me', { waitUntil: 'domcontentloaded', timeout: 40_000 });

    const core = liveIn(page, '.hp-core');
    await expect(core).toBeVisible({ timeout: 20_000 });
    await expect(core).toContainText('0h');
  });
});
