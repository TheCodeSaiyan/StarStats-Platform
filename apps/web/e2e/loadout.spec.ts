/**
 * Loadout paperdoll page + loadout widget e2e tests.
 *
 * ============================================================
 * INFRASTRUCTURE CONSTRAINTS — READ BEFORE ENABLING
 * ============================================================
 *
 * 1. These tests require the mock server + Next dev server.
 *    Run via `pnpm --filter web exec playwright test loadout` —
 *    that starts both via webServer in playwright.config.ts.
 *    A bare `playwright test` without running infra ECONNREFUSED.
 *
 * 2. All API calls go Next.js server → mock at port 3199.
 *    Browser-side page.route() cannot intercept server-to-server
 *    fetches from RSC. setScenario() is the correct mechanism.
 *
 * 3. MOCK ROUTES REQUIRED for the /me/loadout page:
 *    POST /v1/reference/resolve  → resolvedMap (below, overrides base)
 *    GET  /v1/me/events          → eventsWithBurst (below, overrides base)
 *    Plus all base routes from scenarioFor (layout, auth, etc.)
 *
 * ============================================================
 */

import { expect, test } from '@playwright/test';
import {
  loginAs,
  resetScenario,
  scenarioFor,
  setScenario,
} from './helpers/api-mock';

// ---------------------------------------------------------------------------
// Shared fixture data
// ---------------------------------------------------------------------------

/** A burst_summary event with two visible items and one excluded anatomy port. */
const eventsWithBurst = {
  status: 200,
  body: {
    events: [
      {
        seq: 99,
        source_offset: 99 * 1024,
        log_source: 'live',
        event_type: 'burst_summary',
        event_timestamp: '2026-06-24T00:00:00Z',
        payload: {
          kind: 'loadout_restore',
          items: [
            { class: 'GRIN_Light_Helmet', port: 'head_attach', category: 'item' },
            { class: 'BEHR_P4AR', port: 'weapon_attach_0', category: 'weapon' },
            // anatomy cosmetic — excluded by isExcludedPort
            { class: 'SomeThing', port: 'eyes_itemport', category: 'item' },
          ],
        },
      },
    ],
    next_after: null,
  },
};

/** Resolve response returning a friendly name for the helmet. */
const resolvedMap = {
  status: 200,
  body: {
    names: { GRIN_Light_Helmet: 'Light Helmet' },
    resolved: {
      GRIN_Light_Helmet: {
        display_name: 'Light Helmet',
        slug: 'light-helmet',
        category: 'item',
        classification: 'FPS.Armor.Helmet',
        classification_label: 'Helmet',
        has_image: false,
      },
      BEHR_P4AR: {
        display_name: 'Ballistic Pistol',
        slug: 'ballistic-pistol',
        category: 'weapon',
        classification: 'FPS.Weapon.Pistol',
        classification_label: 'Pistol',
        has_image: false,
      },
    },
  },
};

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

test.beforeEach(async ({ request }) => {
  await resetScenario(request);
});

test('loadout_page_renders_body_outline_and_gear_group', async ({
  page,
  request,
}) => {
  await loginAs(page, { handle: 'TestPilot' });
  await setScenario(
    request,
    scenarioFor('loadout_page_renders', {
      'GET /v1/me/events': eventsWithBurst,
      'POST /v1/reference/resolve': resolvedMap,
    }),
  );

  await page.goto('/me/loadout');

  // Page title is present
  await expect(
    page.getByRole('heading', { name: /loadout/i }),
  ).toBeVisible();

  // Body outline renders (has slot placeholders / filled slots)
  // The BodyOutline has class="body-outline"; at minimum Head slot
  // resolves to "Light Helmet" (filled) or a placeholder
  // `.hp-paperdoll` since the projection port — same paperdoll, beam markup.
  await expect(page.locator('.hp-paperdoll')).toBeVisible();

  // At least one gear group is rendered — Weapons group from BEHR_P4AR
  // Gear groups are flat Planes now, each captioned with its group name.
  await expect(page.locator('.hp-geargrid').first()).toBeVisible();

  // The Weapons group heading is present
  await expect(page.getByRole('heading', { name: /weapons/i })).toBeVisible();
});

test('loadout_page_shows_empty_state_when_no_burst', async ({
  page,
  request,
}) => {
  await loginAs(page, { handle: 'TestPilot' });
  await setScenario(
    request,
    scenarioFor('loadout_page_empty', {
      'GET /v1/me/events': { status: 200, body: { events: [], next_after: null } },
    }),
  );

  await page.goto('/me/loadout');

  await expect(page.getByText(/no loadout snapshot/i)).toBeVisible();
});

test('profile_loadout_widget_shows_view_loadout_link', async ({
  page,
  request,
}) => {
  await loginAs(page, { handle: 'TestPilot' });
  await setScenario(
    request,
    scenarioFor('me_loadout_widget_view_link', {
      'GET /v1/me/events': eventsWithBurst,
      'POST /v1/reference/resolve': resolvedMap,
      // The loadout widget is opt-in (disabled in DEFAULT_LAYOUT), so the
      // widget canvas only renders it when the stored profile-layout
      // enables it. Without this the widget — and its "View loadout"
      // link — never mounts on /me.
      'GET /v1/users/me/profile-layout': {
        status: 200,
        body: { layout: [{ id: 'loadout', enabled: true, size: 'compact' }] },
      },
      // `/u/[handle]` as the OWNER: page.tsx short-circuits to the self
      // path before hitting the public endpoints, so these are
      // belt-and-suspenders stubs that keep the scenario deterministic.
      'GET /v1/public/TestPilot/summary': { status: 404, body: {} },
      'GET /v1/public/TestPilot/rsi-profile': { status: 404, body: {} },
      'GET /v1/public/TestPilot/rsi-orgs': { status: 200, body: { orgs: [] } },
    }),
  );

  // `/u/[handle]`, not `/me`: the projection replaced the widget grid on /me,
  // and the loadout WIDGET (as opposed to the `/me/loadout` page, covered
  // above) now only renders on the profile surface.
  await page.goto('/u/TestPilot');

  // The loadout widget renders the "View loadout →" link
  await expect(
    page.getByRole('link', { name: /view loadout/i }),
  ).toBeVisible();
});
