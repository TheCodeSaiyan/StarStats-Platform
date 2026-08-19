/**
 * /sharing — Profile views card (Piece 2 of public-profile UX).
 *
 * The card lives directly under the visibility toggle. When the user
 * is public, it shows the 30-day view total, a per-source breakdown
 * line, and a sparkline of per-day counts. When private it collapses
 * to a placeholder pointing at the toggle.
 *
 * We mock /v1/me/profile-views from the shared mock server; the
 * dev-server-side Next.js fetcher routes through the same mock host
 * because `STARSTATS_API_URL` points at it.
 */

import { expect, test } from '@playwright/test';
import {
  loginAs,
  resetScenario,
  scenarioFor,
  setScenario,
} from './helpers/api-mock';

test.beforeEach(async ({ request, page }) => {
  await resetScenario(request);
  await loginAs(page);
});

test('profile_views_card_renders_count_and_breakdown_when_public', async ({
  page,
  request,
}) => {
  // Seed: profile public, /v1/me/profile-views returns 23 last_30d
  // split 12/8/3 across direct/discover/shared plus a sparkline.
  await setScenario(
    request,
    scenarioFor('sharing_views_public', {
      'GET /v1/me/visibility': { status: 200, body: { public: true } },
      'GET /v1/me/profile-views': {
        status: 200,
        body: {
          days: [
            { day: '2026-05-18', total: 6, by_source: { direct: 4, discover: 2 } },
            { day: '2026-05-17', total: 9, by_source: { direct: 5, shared: 4 } },
            { day: '2026-05-16', total: 8, by_source: { direct: 3, discover: 5 } },
          ],
          totals: {
            all_time: 41,
            last_7d: 15,
            last_30d: 23,
            by_source_30d: { direct: 12, discover: 8, shared: 3 },
          },
        },
      },
    }),
  );

  await page.goto('/sharing');

  const card = page.getByTestId('profile-views-card');
  await expect(card).toBeVisible();
  // Big number = last_30d.
  await expect(card.getByTestId('profile-views-total')).toHaveText('23');
  const breakdown = card.getByTestId('profile-views-breakdown');
  await expect(breakdown).toContainText('12 from direct links');
  await expect(breakdown).toContainText('8 from discover');
  await expect(breakdown).toContainText('3 from shared profiles');
  // Sparkline renders one bar per seeded day (3 days here).
  await expect(card.getByTestId('profile-views-sparkline')).toBeVisible();
});

test('profile_views_card_shows_empty_state_when_no_views_yet', async ({
  page,
  request,
}) => {
  await setScenario(
    request,
    scenarioFor('sharing_views_empty', {
      'GET /v1/me/visibility': { status: 200, body: { public: true } },
      'GET /v1/me/profile-views': {
        status: 200,
        body: {
          days: [],
          totals: {
            all_time: 0,
            last_7d: 0,
            last_30d: 0,
            by_source_30d: {},
          },
        },
      },
    }),
  );

  await page.goto('/sharing');

  const card = page.getByTestId('profile-views-card');
  await expect(card).toBeVisible();
  await expect(card.getByText('No views yet.')).toBeVisible();
});

test('profile_views_card_collapses_to_placeholder_when_private', async ({
  page,
  request,
}) => {
  // Default scenario already returns `public: false` from
  // /v1/me/visibility. The card should NOT fetch views, but in case
  // the page-level Promise.allSettled still hits the endpoint, return
  // a benign empty body so the test isn't sensitive to the call.
  await setScenario(
    request,
    scenarioFor('sharing_views_private', {
      'GET /v1/me/profile-views': {
        status: 200,
        body: {
          days: [],
          totals: {
            all_time: 0,
            last_7d: 0,
            last_30d: 0,
            by_source_30d: {},
          },
        },
      },
    }),
  );

  await page.goto('/sharing');

  const card = page.getByTestId('profile-views-card');
  await expect(card).toBeVisible();
  await expect(
    card.getByText('Make your profile public to start tracking views.'),
  ).toBeVisible();
  // The number / sparkline should NOT render under the private placeholder.
  await expect(card.getByTestId('profile-views-total')).toHaveCount(0);
});
