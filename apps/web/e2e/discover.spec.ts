/**
 * /discover — public-profile listing surface.
 *
 * Piece 3 of the public-profile UX work. The page is unauthenticated
 * by design and renders directly from `GET /v1/discover/profiles`.
 * Tests below mock the upstream and assert:
 *   * empty state when the API returns no profiles
 *   * grid renders profile cards from a populated response
 *   * "Load more" button appears when `next_after` is present
 *   * card links include the `?source=discover` attribution param
 *     so Piece 2's view counter buckets correctly
 */

import { expect, test } from '@playwright/test';
import { resetScenario, setScenario } from './helpers/api-mock';

test.beforeEach(async ({ request }) => {
  await resetScenario(request);
});

test('discover_renders_empty_state_when_api_returns_empty_list', async ({
  page,
  request,
}) => {
  await setScenario(request, {
    __id: 'discover_empty',
    routes: {
      'GET /v1/discover/profiles': {
        status: 200,
        body: { profiles: [], next_after: null },
      },
    },
  });

  await page.goto('/discover');

  await expect(page.getByTestId('discover-empty-state')).toBeVisible();
  // The grid wrapper isn't rendered in the empty case — we render
  // the empty-state block instead, so asserting absence on the grid
  // catches a regression where the empty path forgets to suppress it.
  await expect(page.getByTestId('discover-grid')).toHaveCount(0);
  await expect(page.getByTestId('discover-load-more')).toHaveCount(0);
});

test('discover_renders_profile_cards_when_api_returns_data', async ({
  page,
  request,
}) => {
  await setScenario(request, {
    __id: 'discover_full',
    routes: {
      'GET /v1/discover/profiles': {
        status: 200,
        body: {
          profiles: [
            {
              handle: 'Alice',
              display_name: 'Alice Aviatrix',
              joined_at: '2026-01-01T00:00:00+00:00',
              last_active_at: '2026-05-17T00:00:00+00:00',
            },
            {
              handle: 'Bob',
              display_name: null,
              joined_at: '2026-02-01T00:00:00+00:00',
              last_active_at: null,
            },
          ],
          next_after: null,
        },
      },
    },
  });

  await page.goto('/discover');

  const cards = page.getByTestId('discover-profile-card');
  await expect(cards).toHaveCount(2);
  // The handle is the dominant glyph on each card — assert by data
  // attribute so the assertion stays decoupled from the visual
  // typography token.
  await expect(cards.nth(0)).toHaveAttribute('data-handle', 'Alice');
  await expect(cards.nth(1)).toHaveAttribute('data-handle', 'Bob');
  // Display name is optional; the second card should not surface it.
  await expect(cards.nth(0)).toContainText('Alice Aviatrix');
  await expect(cards.nth(1)).not.toContainText('Aviatrix');
});

test('discover_shows_load_more_when_next_after_is_present', async ({
  page,
  request,
}) => {
  await setScenario(request, {
    __id: 'discover_paginated',
    routes: {
      'GET /v1/discover/profiles': {
        status: 200,
        body: {
          profiles: [
            {
              handle: 'Alice',
              display_name: null,
              joined_at: '2026-01-01T00:00:00+00:00',
              last_active_at: null,
            },
          ],
          // Non-null cursor → the page must mount the load-more
          // button so the user can walk the next slice.
          next_after: 'alice',
        },
      },
    },
  });

  await page.goto('/discover');

  await expect(page.getByTestId('discover-load-more')).toBeVisible();
  await expect(page.getByTestId('discover-load-more')).toHaveText('Load more');
});

test('discover_profile_card_link_carries_source_attribution', async ({
  page,
  request,
}) => {
  // The `?source=discover` query param feeds Piece 2's view-counter
  // recorder. A regression that drops it would attribute /discover
  // clicks to `Other`, silently breaking the per-source breakdown
  // on the /sharing profile-views card.
  await setScenario(request, {
    __id: 'discover_attribution',
    routes: {
      'GET /v1/discover/profiles': {
        status: 200,
        body: {
          profiles: [
            {
              handle: 'Charlie',
              display_name: null,
              joined_at: '2026-01-01T00:00:00+00:00',
              last_active_at: null,
            },
          ],
          next_after: null,
        },
      },
    },
  });

  await page.goto('/discover');

  const card = page.getByTestId('discover-profile-card').first();
  const href = await card.getAttribute('href');
  expect(href).toBe('/u/Charlie?source=discover');
});
