/**
 * H4: the signed-in TopBar fans out to two layout-level fetches on every
 * render — `GET /v1/me/location/current` (the location chip) and
 * `GET /v1/me/shared-with-me` (the AccountMenu inbound-share badge).
 * Both now have base fixtures in `scenarioFor()` so no scenario emits a
 * `599 no_mock_fixture`. This spec proves the two surfaces actually render
 * when the fetches return populated data.
 */

import { expect, test } from '@playwright/test';
import { loginAs, resetScenario, setScenario, scenarioFor } from './helpers/api-mock';

const HANDLE = 'TestPilot';

test.describe('TopBar layout fetches (H4)', () => {
  test.beforeEach(async ({ request, page }) => {
    await resetScenario(request);
    await loginAs(page, { handle: HANDLE });
  });

  test('renders the current-location chip and the inbound-share badge', async ({
    page,
    request,
  }) => {
    await setScenario(
      request,
      scenarioFor('topbar_layout_fetches', {
        // Populated current location → LocationChip headline = city.
        'GET /v1/me/location/current': {
          status: 200,
          body: {
            location: {
              last_seen_at: '2026-05-22T12:00:00Z',
              source_event_type: 'planet_terrain_load',
              city: 'Orison',
              planet: 'Crusader',
              system: 'Stanton',
            },
          },
        },
        // One active inbound share (no expires_at ⇒ counted) → badge "1 new".
        'GET /v1/me/shared-with-me': {
          status: 200,
          body: {
            shared_with_me: [{ owner_handle: 'friendpilot' }],
          },
        },
      }),
    );

    // `/settings` renders the signed-in TopBar without the `/me` widget
    // canvas, isolating the two layout-level fetches under test.
    await page.goto('/settings');

    // Location chip headline is `city ?? planet ?? 'In transit'`.
    await expect(page.getByText('Orison').first()).toBeVisible();
    // Inbound-share badge in AccountMenu carries aria-label `${n} new`.
    await expect(page.getByLabel('1 new')).toBeVisible();
  });
});
