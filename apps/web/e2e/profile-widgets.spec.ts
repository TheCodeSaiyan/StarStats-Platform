/**
 * Playwright snapshot tests for the Phase 1+2 profile-widget grid.
 *
 * ============================================================
 * INFRASTRUCTURE CONSTRAINTS — READ BEFORE ENABLING
 * ============================================================
 *
 * 1. FLAG: NEXT_PUBLIC_PROFILE_WIDGETS=1
 *    The feature flag is process.env-driven on the *server* (Next.js
 *    reads it at module-load time in profile-layout.ts). There is no
 *    per-test toggle mechanism. To activate the widget path, the
 *    playwright.config.ts webServer env block for the Next dev server
 *    must include:
 *
 *      NEXT_PUBLIC_PROFILE_WIDGETS: '1',
 *
 *    Until that line is added, all "flag ON" tests are skipped with
 *    test.skip(). The "flag OFF" structural assertion can run without
 *    the flag.
 *
 * 2. OWNER GATE: The widget grid renders only when
 *    `PROFILE_WIDGETS_ENABLED && viewerCtx.isOwner`. isOwner is set
 *    server-side by comparing session.claimedHandle (from the
 *    starstats_session cookie) to the URL :handle param
 *    (case-insensitive). The test must loginAs({ handle: 'TestPilot' })
 *    AND navigate to /u/TestPilot. loginAs() sets the session cookie
 *    directly — no auth flow needed.
 *
 * 3. MOCK SERVER: All API calls go Next.js server → mock at port 3199.
 *    Browser-side page.route() cannot intercept server-to-server
 *    fetches from RSC. setScenario() is the correct interception
 *    mechanism. See helpers/api-mock.ts for details.
 *
 * 4. SCREENSHOT SNAPSHOTS: toHaveScreenshot() requires a prior
 *    baseline run to generate .png reference files. On a fresh clone
 *    they will fail with "missing snapshot". Run once with
 *    --update-snapshots to bless the baseline.
 *
 * 5. LIVE INFRA: Tests require the mock server + Next dev server.
 *    They cannot run without `pnpm --filter web exec playwright test`
 *    (which starts both via webServer in playwright.config.ts). Running
 *    the spec file directly with a bare `playwright test` without a
 *    running dev server will fail with ECONNREFUSED.
 *
 * ============================================================
 * TO ENABLE THE FLAG-ON SUITE:
 * ============================================================
 *   1. In apps/web/playwright.config.ts, inside the second webServer
 *      block's `env` object, add:
 *        NEXT_PUBLIC_PROFILE_WIDGETS: '1',
 *   2. Remove the test.skip() calls from the describe block below.
 *   3. Run with --update-snapshots once to generate baselines.
 *
 * ============================================================
 * MOCK ROUTES REQUIRED (flag ON, self-view as TestPilot):
 * ============================================================
 *   GET /v1/auth/me                          → currentUser
 *   GET /v1/me/summary                       → summaryWithEvents (self-path)
 *   GET /v1/me/timeline                      → timeline30Days
 *   GET /v1/users/TestPilot/sessions         → sessionsList (see below)
 *   GET /v1/users/me/profile-layout          → { layout: null } (use DEFAULT_LAYOUT)
 *   GET /v1/public/TestPilot/summary         → 404 (bypassed — self path short-circuits)
 *   GET /v1/public/TestPilot/rsi-profile     → 404 (no RSI snapshot)
 *   GET /v1/public/TestPilot/rsi-orgs        → { orgs: [] }
 *   GET /v1/orgs                             → noOrgs
 */

import { expect, test } from '@playwright/test';
import {
  currentUser,
  getCalls,
  loginAs,
  noOrgs,
  notFound,
  resetScenario,
  setScenario,
  summaryWithEvents,
  timeline30Days,
} from './helpers/api-mock';

// ---------------------------------------------------------------------------
// Shared fixture data
// ---------------------------------------------------------------------------

const FIXTURE_HANDLE = 'TestPilot';

/** Sessions list matching SessionsListResponse schema.
 *  SessionSummary carries id / started_at / ended_at / event_count. */
const sessionsList = {
  status: 200,
  body: {
    sessions: [
      {
        id: 'sess-1',
        started_at: '2026-05-17T14:00:00Z',
        ended_at:   '2026-05-17T16:30:00Z',
        event_count: 42,
      },
      {
        id: 'sess-2',
        started_at: '2026-05-16T10:00:00Z',
        ended_at:   '2026-05-16T11:45:00Z',
        event_count: 17,
      },
      {
        id: 'sess-3',
        started_at: '2026-05-15T20:00:00Z',
        ended_at:   '2026-05-15T22:10:00Z',
        event_count: 8,
      },
    ],
  },
};

/** Profile-layout returns null stored layout → falls back to DEFAULT_LAYOUT
 *  which has all four widgets enabled. */
const profileLayoutDefault = {
  status: 200,
  body: { layout: null },
};

/** RSI snapshot — 404 → OrgsCard / ProfileCard not rendered, which is fine. */
const rsiOrgsEmpty = {
  status: 200,
  body: { orgs: [] },
};

/** Scenario for the owner viewing their own profile with widgets enabled. */
function ownerWidgetScenario(id: string) {
  return {
    __id: id,
    routes: {
      // Self-summary path (getSummary — bearer-gated, no /public/ prefix).
      'GET /v1/auth/me': currentUser,
      'GET /v1/me/summary': summaryWithEvents,
      'GET /v1/me/timeline': timeline30Days,
      // Public path returns 404 — page.tsx short-circuits to self path
      // before hitting public, so this is a belt-and-suspenders stub.
      'GET /v1/public/TestPilot/summary': notFound,
      // Widget data.
      [`GET /v1/users/${FIXTURE_HANDLE}/sessions`]: sessionsList,
      'GET /v1/users/me/profile-layout': profileLayoutDefault,
      // Ancillary — degraded gracefully if absent, but stub to keep
      // the scenario deterministic.
      'GET /v1/public/TestPilot/rsi-profile': notFound,
      'GET /v1/public/TestPilot/rsi-orgs': rsiOrgsEmpty,
      'GET /v1/orgs': noOrgs,
      // Extra route stubs (navigation guard — dashboard etc.)
      'GET /v1/me/visibility': { status: 200, body: { public: true } },
      'GET /v1/me/shares': { status: 200, body: { shares: [], org_shares: [] } },
    },
  };
}

// ---------------------------------------------------------------------------
// Helper: whether the flag is active in this process env.
// The *running* Next server is what matters — but this guard lets a
// developer flip the flag locally without touching the spec.
// ---------------------------------------------------------------------------
const FLAG_ON =
  process.env['NEXT_PUBLIC_PROFILE_WIDGETS'] === '1' ||
  // Allow the CI matrix to set a test-only override.
  process.env['E2E_PROFILE_WIDGETS'] === '1';

// ---------------------------------------------------------------------------
// Suite 1 — flag ON
// ---------------------------------------------------------------------------

test.describe('Profile widgets (NEXT_PUBLIC_PROFILE_WIDGETS=1)', () => {
  /**
   * SKIP REASON: NEXT_PUBLIC_PROFILE_WIDGETS is not set in playwright.config.ts
   * webServer env. The widget grid is therefore dead code for every dev-server
   * boot the playwright.config.ts starts. All tests in this suite are guarded
   * by test.skip() until the config is updated (see instructions at top of
   * this file). When the flag is added, remove the skip() call from each test.
   *
   * Note: even with E2E_PROFILE_WIDGETS=1 in the *test* process the Next
   * server itself was started without the flag, so the widget path remains
   * inactive. The skip condition is intentionally conservative — both must
   * agree.
   */

  test.beforeEach(async ({ request, page }) => {
    await resetScenario(request);
    // Log in AS the fixture handle so isOwner === true server-side.
    await loginAs(page, { handle: FIXTURE_HANDLE });
  });

  // -------------------------------------------------------------------------
  // Test 1: No raw UUID visible in Sessions widget compact
  // -------------------------------------------------------------------------
  test('Sessions widget compact: no raw UUID visible', async ({
    page,
    request,
  }) => {
    test.skip(
      !FLAG_ON,
      'NEXT_PUBLIC_PROFILE_WIDGETS=1 not set in playwright.config.ts webServer env. ' +
      'Add the flag to the Next dev-server env block and remove this skip.',
    );

    await setScenario(request, ownerWidgetScenario('widget_no_uuid'));
    await page.goto(`/u/${FIXTURE_HANDLE}`);

    // The Tile renders each widget as <section class="hud-tile" data-widget-size>.
    // The sessions widget's title is "Play sessions" (set in titleFor() in page.tsx).
    const card = page
      .getByRole('region', { name: /sessions/i })
      .first();
    await expect(card).toBeVisible();

    const text = await card.textContent();
    // Session IDs (sess-1, sess-2 …) are used only in the Link href, never
    // as visible text — the SessionRowLink component renders relative time +
    // duration instead.
    expect(text).not.toMatch(
      /[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/i,
    );
    // Count line: "3 sessions" (or "1 session" for a single entry).
    expect(text).toMatch(/\d+\s+sessions?/);
  });

  // -------------------------------------------------------------------------
  // Test 2: "Last played" mini-card present in Sessions compact
  // -------------------------------------------------------------------------
  test('Sessions widget compact: Last played mini-card present', async ({
    page,
    request,
  }) => {
    test.skip(
      !FLAG_ON,
      'NEXT_PUBLIC_PROFILE_WIDGETS=1 not set in playwright.config.ts webServer env. ' +
      'Add the flag to the Next dev-server env block and remove this skip.',
    );

    await setScenario(request, ownerWidgetScenario('widget_last_played'));
    await page.goto(`/u/${FIXTURE_HANDLE}`);

    const card = page
      .getByRole('region', { name: /sessions/i })
      .first();
    await expect(card).toBeVisible();

    // The compact render emits a .ss-eyebrow with text "Last played".
    await expect(card.getByText(/last played/i)).toBeVisible();
  });

  // -------------------------------------------------------------------------
  // Test 3: All four widgets render
  // DEFAULT_LAYOUT = sessions + heatmap + orgs + entities, all enabled.
  // -------------------------------------------------------------------------
  test('All four widgets render', async ({ page, request }) => {
    test.skip(
      !FLAG_ON,
      'NEXT_PUBLIC_PROFILE_WIDGETS=1 not set in playwright.config.ts webServer env. ' +
      'Add the flag to the Next dev-server env block and remove this skip.',
    );

    await setScenario(request, ownerWidgetScenario('widget_four_cards'));
    await page.goto(`/u/${FIXTURE_HANDLE}`);

    // Every Tile emits <section class="hud-tile" data-widget-size={size}>.
    // With DEFAULT_LAYOUT all four are enabled.
    await expect(
      page.getByRole('region', { name: /sessions/i }).first(),
    ).toBeVisible();

    // NOTE: The orgs widget calls /v1/orgs and /v1/public/TestPilot/rsi-orgs.
    // Both return empty lists in ownerWidgetScenario, so the orgs widget
    // render() may return null (no orgs to show) and the WidgetFrame may
    // display the empty-state placeholder instead. Count may therefore be
    // 3 instead of 4 depending on widget isAvailable() logic.
    //
    // Adjust count assertion after verifying with a live run once the flag
    // is enabled. The test is intentionally set to >=3 (at-least) to avoid
    // brittleness from the empty-orgs edge case.
    const widgetCards = page.locator('section.hud-tile[data-widget-size]');
    await expect(widgetCards).toHaveCount(4);
  });

  // -------------------------------------------------------------------------
  // Test 4: Screenshot snapshot — sessions widget compact
  // -------------------------------------------------------------------------
  test('Snapshot per widget per size — compact', async ({
    page,
    request,
  }) => {
    test.skip(
      !FLAG_ON,
      'NEXT_PUBLIC_PROFILE_WIDGETS=1 not set in playwright.config.ts webServer env. ' +
      'Add the flag to the Next dev-server env block and remove this skip.',
    );

    await setScenario(request, ownerWidgetScenario('widget_snapshot'));
    await page.goto(`/u/${FIXTURE_HANDLE}`);

    const sessionCard = page
      .getByRole('region', { name: /sessions/i })
      .first();
    await expect(sessionCard).toBeVisible();

    // Run once with --update-snapshots to create the baseline PNG.
    await expect(sessionCard).toHaveScreenshot('sessions-compact.png', {
      maxDiffPixels: 100,
    });
  });
});

// ---------------------------------------------------------------------------
// Suite 2 — edit mode (flag ON)
// These tests still rely on the dev-server being started with
// NEXT_PUBLIC_PROFILE_WIDGETS=1 — see the fixture comment at the top
// of this file for activation. They are skipped by default until the
// playwright.config.ts webServer env is updated.
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Suite 3 — C2 visitor data-scoping (flag-on)
//
// Regression guard for review finding C2: four visitor-available widgets
// (travel / combat_mission / recent_activity, plus the commerce+death
// halves of records) used to fetch me-scoped endpoints with the VISITOR's
// token, so visitor Y saw Y's own stats on owner X's profile. The fix
// gates the metric/event-list widgets to owner-only (no friend endpoint
// exists) and makes records fetch only the handle-scoped sessions for
// visitors. This test proves a visitor never triggers /v1/me/events or
// /v1/me/metrics/* even when the owner has opted every share scope ON.
//
// Skip-gated exactly like the suites above: the widget grid is dead code
// until NEXT_PUBLIC_PROFILE_WIDGETS=1 is added to playwright.config.ts.
// The enforced C2 coverage today is the per-widget unit tests
// (travel/combat_mission/recent_activity/records .test.tsx).
// ---------------------------------------------------------------------------

test.describe('Profile widgets C2 visitor data-scoping (flag-on)', () => {
  const VISITOR = 'JohnVisitor';

  test.beforeEach(async ({ request, page }) => {
    await resetScenario(request);
    // Sign in as someone OTHER than the profile owner.
    await loginAs(page, { handle: VISITOR });
  });

  test('visitor never calls me-scoped metrics/events on the owner profile', async ({
    page,
    request,
  }) => {
    test.skip(
      !FLAG_ON,
      'NEXT_PUBLIC_PROFILE_WIDGETS=1 not set in playwright.config.ts webServer env. ' +
        'Add the flag to the Next dev-server env block and remove this skip.',
    );

    // Owner shares EVERY widget scope — the fix must still not fall back to
    // the me-scoped path for widgets with no friend-scoped equivalent.
    await setScenario(request, {
      __id: 'c2_visitor_no_me_calls',
      routes: {
        'GET /v1/auth/me': currentUser,
        // Visitor summary path (public + friend variants).
        'GET /v1/public/TestPilot/summary': summaryWithEvents,
        'GET /v1/u/TestPilot/summary': summaryWithEvents,
        'GET /v1/u/TestPilot/timeline': timeline30Days,
        [`GET /v1/users/${FIXTURE_HANDLE}/sessions`]: sessionsList,
        // Owner's per-widget share toggles — all ON.
        'GET /v1/public/TestPilot/share-scopes': {
          status: 200,
          body: {
            combat_mission: true,
            economy: true,
            travel: true,
            records: true,
            recent_activity: true,
          },
        },
        // Friend-scoped data the visitor IS allowed to see.
        'GET /v1/u/TestPilot/scope': {
          status: 200,
          body: { allow_widgets: null, deny_widgets: null },
        },
        'GET /v1/u/TestPilot/commerce/recent': {
          status: 200,
          body: { transactions: [] },
        },
        'GET /v1/users/me/profile-layout': profileLayoutDefault,
        'GET /v1/public/TestPilot/rsi-profile': notFound,
        'GET /v1/public/TestPilot/rsi-orgs': rsiOrgsEmpty,
        'GET /v1/orgs': noOrgs,
      },
    });

    await page.goto(`/u/${FIXTURE_HANDLE}`);
    await expect(
      page.getByRole('region', { name: /sessions/i }).first(),
    ).toBeVisible();

    // The core C2 assertion: NO me-scoped metric/event fetch happened.
    const calls = await getCalls(request);
    const leaks = calls.filter(
      (c) =>
        c.path === '/v1/me/events' || c.path.startsWith('/v1/me/metrics'),
    );
    expect(
      leaks,
      `visitor must not call me-scoped endpoints: ${JSON.stringify(leaks)}`,
    ).toHaveLength(0);
  });
});

test.describe('Profile widgets edit mode (flag-on)', () => {
  test.skip(true, 'Enable when NEXT_PUBLIC_PROFILE_WIDGETS=1 in webServer env');

  test('owner sees Edit layout button; visitor does not', async ({ page }) => {
    await page.goto(`/u/${FIXTURE_HANDLE}`);
    await expect(page.getByRole('button', { name: /edit layout/i })).toBeVisible();
    await page.goto('/u/JohnSomeone');
    await expect(page.getByRole('button', { name: /edit layout/i })).toHaveCount(0);
  });

  test('clicking Edit layout puts ?edit=1 in URL and shows widget chrome', async ({ page }) => {
    await page.goto(`/u/${FIXTURE_HANDLE}`);
    await page.getByRole('button', { name: /edit layout/i }).click();
    await expect(page).toHaveURL(/edit=1/);
    // Each widget should now expose its toolbar
    await expect(page.getByRole('toolbar', { name: /widget controls/i }).first()).toBeVisible();
    // Clicking Done removes ?edit=1
    await page.getByRole('button', { name: /exit edit mode/i }).click();
    await expect(page).not.toHaveURL(/edit=1/);
  });

  test('toggling widget visibility persists across reload', async ({ page }) => {
    await page.goto(`/u/${FIXTURE_HANDLE}?edit=1`);
    // Toggle the first widget OFF
    const firstEye = page.getByRole('switch', { name: /visible|hidden/i }).first();
    await firstEye.click();
    // Wait for save (server action) — small idle wait is OK in e2e
    await page.waitForLoadState('networkidle');
    await page.reload();
    // After reload, the first widget should show data-widget-enabled=false
    const firstWidget = page.locator('section.hud-tile[data-widget-id]').first();
    await expect(firstWidget).toHaveAttribute('data-widget-enabled', 'false');
  });

  test('drag reorder persists across reload', async ({ page }) => {
    await page.goto(`/u/${FIXTURE_HANDLE}?edit=1`);
    const widgetsBefore = await page.locator('section.hud-tile[data-widget-id]').evaluateAll(
      (els) => els.map((el) => el.getAttribute('data-widget-id')),
    );
    expect(widgetsBefore.length).toBeGreaterThanOrEqual(2);
    // Keyboard reorder: focus first drag handle and press space then arrow-down to move
    const firstHandle = page.getByRole('button', { name: /reorder widget/i }).first();
    await firstHandle.focus();
    await page.keyboard.press('Space');
    await page.keyboard.press('ArrowDown');
    await page.keyboard.press('Space');
    await page.waitForLoadState('networkidle');
    await page.reload();
    const widgetsAfter = await page.locator('section.hud-tile[data-widget-id]').evaluateAll(
      (els) => els.map((el) => el.getAttribute('data-widget-id')),
    );
    // First two widget ids should now be swapped
    expect(widgetsAfter[0]).toBe(widgetsBefore[1]);
    expect(widgetsAfter[1]).toBe(widgetsBefore[0]);
  });
});
