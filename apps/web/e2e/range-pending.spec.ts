import { test, expect } from '@playwright/test';
import {
  loginAs,
  resetScenario,
  scenarioFor,
  setScenario,
} from './helpers/api-mock';
import { liveIn } from './helpers/shell';

/**
 * The range control has to show it was clicked.
 *
 * Changing the range is a navigation: `/me?range=…` re-renders the whole
 * dashboard on the server. A bare `<Link>` gives no feedback while that is in
 * flight, so the control read as dead — reported as "I click it, nothing
 * happens, I click it several times and then a few seconds later the time
 * range changes".
 *
 * The repeated clicking is the part that matters. Each click starts another
 * full server render, so an unresponsive-looking control makes itself slower
 * the more it is used.
 *
 * `delayMs` holds an upstream response open, which reproduces a slow render
 * deterministically rather than hoping the machine is loaded.
 */
test.describe('range tabs show pending', () => {
  test.beforeEach(async ({ request }) => {
    await resetScenario(request);
  });

  test('marks the clicked tab pending while the server re-renders', async ({
    page,
    request,
  }) => {
    await setScenario(
      request,
      scenarioFor('range-pending', {
        // Slow ONE range-scoped endpoint. The navigation cannot complete
        // until it resolves, which is the window the reader is staring at.
        'GET /v1/me/stats/playtime': {
          status: 200,
          body: { total_playtime_secs: 3600, session_count: 2 },
          delayMs: 4000,
        },
      }),
    );
    await loginAs(page, { handle: 'TestPilot' });
    await page.setViewportSize({ width: 1600, height: 950 });
    await page.goto('/me', { waitUntil: 'domcontentloaded', timeout: 40_000 });

    const tabs = liveIn(page, '.hp-rng');
    await expect(tabs).toBeVisible({ timeout: 20_000 });

    const target = tabs.getByRole('link', { name: '30d' });
    await target.click();

    // The clicked tab — and only the clicked tab — reports itself pending.
    await expect(
      target.locator('[data-pending]'),
      'the control must show it was clicked, or readers click again and each click costs a render',
    ).toBeVisible({ timeout: 5_000 });
    await expect(tabs.locator('a [data-pending]')).toHaveCount(1);
  });

  test('clears the pending mark once the range has changed', async ({
    page,
    request,
  }) => {
    await setScenario(request, scenarioFor('range-pending-fast'));
    await loginAs(page, { handle: 'TestPilot' });
    await page.setViewportSize({ width: 1600, height: 950 });
    await page.goto('/me', { waitUntil: 'domcontentloaded', timeout: 40_000 });

    const tabs = liveIn(page, '.hp-rng');
    await expect(tabs).toBeVisible({ timeout: 20_000 });
    await tabs.getByRole('link', { name: '30d' }).click();

    await expect(page).toHaveURL(/range=30d/, { timeout: 30_000 });
    // A pending mark that outlives the navigation would be worse than none.
    await expect(tabs.locator('a [data-pending]')).toHaveCount(0, {
      timeout: 15_000,
    });
  });
});
