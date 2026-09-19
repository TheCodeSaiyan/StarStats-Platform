import { test, expect } from '@playwright/test';
import {
  getCalls,
  loginAs,
  resetScenario,
  scenarioFor,
  setScenario,
} from './helpers/api-mock';

/**
 * One render, one call per endpoint.
 *
 * `/me` fans out ~37 API calls across 23 widgets, and several of those
 * widgets want the SAME endpoint: `journey` and `corridors` both read the
 * location trace, `travel` and `routes` both read routes, `combat_mission`
 * and `objectives` both read objectives, `economy` and `spend` both read
 * spend. Every request is `cache: 'no-store'` (api.ts), so there is no data
 * cache to collapse them — each duplicate is a real HTTP round trip taking a
 * connection from a pool of 16 with a 5s acquire timeout.
 *
 * `lib/reference.ts` already fixed exactly this for the KB and wrote down the
 * symptom: "Beta's logs showed every slug fetched exactly twice, which is
 * this." The remedy there — React `cache()`, request-scoped, dedupes within a
 * render and never leaks between readers — simply had not been applied to the
 * dashboard fetchers.
 */
test.describe('dashboard fetch dedupe', () => {
  test.beforeEach(async ({ request }) => {
    await resetScenario(request);
    await setScenario(request, scenarioFor('fetch-dedupe'));
  });

  test('renders /me without fetching any endpoint twice', async ({
    page,
    request,
  }) => {
    await loginAs(page, { handle: 'TestPilot' });
    await page.setViewportSize({ width: 1600, height: 950 });
    await page.goto('/me', { waitUntil: 'domcontentloaded', timeout: 40_000 });
    await expect(page.locator('.hp-lens button').first()).toBeVisible({
      timeout: 20_000,
    });

    const calls = await getCalls(request);
    // Key on method + path + query: the same endpoint at a DIFFERENT window is
    // a different question and must still be allowed through.
    const seen = new Map<string, number>();
    for (const c of calls) {
      const key = `${c.method} ${c.path}${c.query ? `?${c.query}` : ''}`;
      seen.set(key, (seen.get(key) ?? 0) + 1);
    }
    const duplicated = [...seen.entries()]
      .filter(([, n]) => n > 1)
      .map(([k, n]) => `${n}× ${k}`)
      .sort();

    expect(
      duplicated,
      'every duplicate here is a wasted round trip and a wasted DB connection',
    ).toEqual([]);
  });
});
