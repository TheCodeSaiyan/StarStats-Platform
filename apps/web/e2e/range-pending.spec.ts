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

/**
 * The selected range must survive a hop between pages.
 *
 * The range is URL state deliberately — the tabs are `<Link>`s so the back
 * button stays correct and a view is shareable. But every link BETWEEN pages
 * was a bare path, so picking 90d on `/me` and following a "see all" landed on
 * the default 7d, and going back landed on 7d again.
 *
 * That is worse than a cosmetic reset. A reader whose events are months old
 * sees an empty page and concludes the data is missing, rather than that the
 * window quietly changed under them — which is exactly how a real ship-loss
 * count got reported as "not coming through".
 */
test.describe('range survives navigation', () => {
  test.beforeEach(async ({ request }) => {
    await resetScenario(request);
    await setScenario(request, scenarioFor('range-nav'));
  });

  test('carries the chosen range through a see-all link', async ({ page }) => {
    test.slow();
    await loginAs(page, { handle: 'TestPilot' });
    await page.setViewportSize({ width: 1600, height: 950 });
    await page.goto('/me?range=90d', {
      waitUntil: 'domcontentloaded',
      timeout: 40_000,
    });

    const tabs = liveIn(page, '.hp-rng');
    await expect(tabs).toBeVisible({ timeout: 20_000 });

    // Every link that leads to a WINDOWED page must carry the range. A bare
    // `/me/contracts` here is the bug.
    // Unconditional: these links are always rendered on /me, so a missing
    // one is a failure rather than a skipped assertion. The earlier shape of
    // this test guarded every expect with `if (count())`, which passes
    // happily when the link is absent - the exact vacuous pass that lets a
    // nav regression through.
    const contracts = page.locator('a[href*="/me/contracts"]').first();
    await expect(contracts).toHaveAttribute('href', /range=90d/);

    const travel = page.locator('a[href*="/me/travel"]').first();
    await expect(travel).toHaveAttribute('href', /range=90d/);

    // And a page with no window must NOT carry one - a parameter there would
    // imply a filter the page does not apply.
    const loadout = page.locator('a[href*="/me/loadout"]').first();
    await expect(loadout).toHaveAttribute('href', '/me/loadout');
  });
});
