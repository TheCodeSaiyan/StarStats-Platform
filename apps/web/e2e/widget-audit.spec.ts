/**
 * Widget dashboard AUDIT harness — NOT a CI test.
 *
 * Renders the signed-in owner dashboard (/me) with EVERY widget enabled and
 * populated, then measures each tile for the three failure modes the owner
 * reported: content CLIPPED (hidden with no scroll), content SCROLLS, and
 * WASTED empty space (tile much taller than its content). Also captures a
 * full-page screenshot artifact.
 *
 * Ground-truth tool for the widget-module v2 work — run before and after to
 * prove the fix. Skipped unless AUDIT=1.
 *
 *   cd apps/web && AUDIT=1 pnpm exec playwright test e2e/widget-audit.spec.ts --project=chromium
 *
 * Out (scratchpad): widget-audit.png + widget-audit.json
 */
import { test, expect } from '@playwright/test';
import { mkdirSync, writeFileSync } from 'node:fs';
import { loginAs, resetScenario, scenarioFor, setScenario } from './helpers/api-mock';

const RUN = process.env.AUDIT === '1';
const OUT_DIR =
  process.env.AUDIT_OUT ??
  'reports/widget-audit';

const s200 = (body: unknown) => ({ status: 200, body });

// Every registry widget id, all enabled. Sizes chosen to reproduce the
// reported symptoms: travel/journey/heatmap EXPANDED (the dense, clip-prone
// ones), the rest compact (where empty-space waste shows).
const ALL_WIDGETS_LAYOUT = [
  { id: 'sessions', enabled: true, size: 'compact' },
  { id: 'heatmap', enabled: true, size: 'expanded' },
  { id: 'orgs', enabled: true, size: 'compact' },
  { id: 'recent_activity', enabled: true, size: 'compact' },
  { id: 'combat_mission', enabled: true, size: 'compact' },
  { id: 'economy', enabled: true, size: 'compact' },
  { id: 'travel', enabled: true, size: 'expanded' },
  { id: 'journey', enabled: true, size: 'expanded' },
  { id: 'records', enabled: true, size: 'compact' },
  { id: 'hangar', enabled: true, size: 'compact' },
  { id: 'loadout', enabled: true, size: 'compact' },
  { id: 'entities', enabled: true, size: 'compact' },
  { id: 'lives', enabled: true, size: 'compact' },
  { id: 'fleet', enabled: true, size: 'compact' },
  { id: 'docking', enabled: true, size: 'compact' },
  { id: 'objectives', enabled: true, size: 'compact' },
  { id: 'spend', enabled: true, size: 'compact' },
  { id: 'routes', enabled: true, size: 'compact' },
  { id: 'locations', enabled: true, size: 'compact' },
];

// Fixtures for every widget endpoint (lifted/adapted from screenshots.spec.ts
// richDashboard + the 6 endpoints it lacked: objectives, spend, travel,
// routes, location/trace, orgs).
const FIXTURES: Record<string, { status: number; body: unknown }> = {
  'GET /v1/users/me/profile-layout': s200({ layout: ALL_WIDGETS_LAYOUT }),
  'GET /v1/me/stats/lives': s200({
    total_lives: 47, deaths: 39, sessions: 62, deaths_per_session: 0.6,
    lives_ended_by_crash: 4, longest_life_secs: 133200, mean_life_secs: 5400, recent_lives: [],
  }),
  'GET /v1/me/stats/fleet': s200({
    ships: [
      { vehicle_class: 'RSI_Constellation_Andromeda', trip_count: 84 },
      { vehicle_class: 'AEGS_Gladius', trip_count: 51 },
      { vehicle_class: 'DRAK_Cutlass_Black', trip_count: 33 },
      { vehicle_class: 'MISC_Prospector', trip_count: 21 },
      { vehicle_class: 'ANVL_Carrack', trip_count: 15 },
      { vehicle_class: 'CRUS_Spirit_C1', trip_count: 12 },
      { vehicle_class: 'RSI_Constellation_Taurus', trip_count: 9 },
      { vehicle_class: 'DRAK_Cutlass_Red', trip_count: 6 },
    ],
  }),
  'GET /v1/me/stats/docking': s200({
    total_stows: 120, by_kind: { hangar: 80, pad: 30, other: 10 },
    by_size: { small: 40, medium: 45, large: 25, xl: 10, unknown: 0 },
  }),
  'GET /v1/me/stats/playtime': s200({ hours: 418, session_count: 62, total_playtime_secs: 418 * 3600 }),
  'GET /v1/me/stats/locations': s200({
    hours: 720, unique_locations: 67,
    top_locations: [
      { value: 'Stanton|microTech|', count: 3886 }, { value: 'Stanton|Hurston|', count: 456 },
      { value: 'Stanton|Clio|', count: 328 }, { value: 'Stanton|microTech|New Babbage', count: 173 },
      { value: '||', count: 32 }, { value: 'PRIMIC_L1', count: 18 },
    ],
  }),
  'GET /v1/me/stats/biggest-trade': s200({ quantity: 384, item: 'Titanium' }),
  'GET /v1/users/TestPilot/sessions': s200({
    sessions: [
      { id: 'sess-1', started_at: '2026-07-21T14:00:00Z', ended_at: '2026-07-21T16:30:00Z', event_count: 142 },
      { id: 'sess-2', started_at: '2026-07-20T10:00:00Z', ended_at: '2026-07-20T13:15:00Z', event_count: 97 },
    ],
  }),
  'GET /v1/me/stats/records': s200({
    longest_session_secs: 21600, busiest_session_events: 512,
    longest_survival_streak_secs: 187200, deadliest_session_deaths: 7,
  }),
  'GET /v1/me/commerce/recent': s200({
    transactions: [
      { kind: 'shop', status: 'confirmed', item: 'Ballistic Gatling', quantity: 1, raw_request: 'buy', started_at: '2026-07-15T18:00:00Z', confirmed_at: '2026-07-15T18:00:05Z', shop_id: 'shop_dumpers' },
      { kind: 'commodity_buy', status: 'confirmed', item: 'Laranite', quantity: 96, raw_request: 'buy', started_at: '2026-07-15T12:00:00Z' },
      { kind: 'commodity_sell', status: 'pending', item: 'Laranite', quantity: 96, raw_request: 'sell', started_at: '2026-07-16T09:00:00Z' },
    ],
  }),
  'GET /v1/me/hangar': s200({
    captured_at: '2026-07-17T08:00:00Z',
    ships: [
      { name: 'Constellation Andromeda', manufacturer: 'RSI', kind: 'ship' },
      { name: 'Gladius', manufacturer: 'Aegis Dynamics', kind: 'ship' },
      { name: 'Cutlass Black', manufacturer: 'Drake Interplanetary', kind: 'ship' },
    ],
  }),
  'GET /v1/me/metrics/event-types': s200({
    types: [
      { event_type: 'join_pu', count: 779 }, { event_type: 'change_server', count: 3424 },
      { event_type: 'seed_solar_system', count: 1860 }, { event_type: 'resolve_spawn', count: 925 },
      { event_type: 'quantum_target_selected', count: 7436 }, { event_type: 'planet_terrain_load', count: 65981 },
      { event_type: 'vehicle_stowed', count: 14314 }, { event_type: 'player_death', count: 34 },
      { event_type: 'vehicle_destruction', count: 22 }, { event_type: 'mission_start', count: 48 },
      { event_type: 'mission_end', count: 41 },
    ],
  }),
  // --- 6 endpoints not fixtured anywhere else ---
  'GET /v1/me/stats/objectives': s200({ total: 42, completed: 29, in_progress: 6, failed: 7, completion_rate: 0.69 }),
  'GET /v1/me/stats/spend': s200({ total_auec: 72364, purchases: 4, sells: 0, top_shop: 'Dumpers Depot' }),
  'GET /v1/me/stats/travel': s200({
    quantum_jumps: 7436, planets_visited: ['Crusader', 'Hurston', 'microTech', 'ArcCorp'],
    top_destinations: [
      { value: 'MIC-L1', count: 40 }, { value: 'Everus Harbor', count: 33 },
      { value: 'ARC-L1', count: 22 }, { value: 'Port Tressler', count: 18 }, { value: 'CRU-L1', count: 12 },
    ],
  }),
  'GET /v1/me/stats/routes': s200({
    routes: [
      { destination: 'LOC_RR_S1_L1', count: 40 }, { destination: 'LOC_RR_S4_L1', count: 33 },
      { destination: 'MISSION_QT_Quantum_Beacon_718368901207', count: 12 },
      { destination: 'MISSION_QT_Quantum_Beacon_718384911828', count: 10 },
      { destination: 'MISSION_QT_Quantum_Beacon_ShortRange_Salvage_720552809368', count: 8 },
      { destination: 'NewBabbage_LOC', count: 22 }, { destination: 'Stanton|microTech|New Babbage', count: 5 },
    ],
  }),
  'GET /v1/me/location/trace': s200({ entries: [] }),
  'GET /v1/public/u/TestPilot/orgs': s200({
    captured_at: '2026-07-17T00:00:00Z',
    orgs: [{ name: 'Test Squadron', sid: 'TESTSQDN', rank: 'Member', member_count: 142, logo_url: null, is_main: true }],
  }),
  // Loadout burst + resolve (populated paperdoll).
  'GET /v1/me/events': s200({
    next_after: null,
    events: [{
      event_timestamp: '2026-07-18T20:14:03Z', event_type: 'burst_summary', hidden_at: null,
      log_source: 'game.log', seq: 91422, source_offset: 5540123, resolved_location: null,
      payload: { kind: 'loadout_restore', items: [
        { class: 'grin_ballistic_helmet_01_black', port: 'armor_helmet', category: 'Char_Armor_Helmet' },
        { class: 'grin_ballistic_core_01_black', port: 'armor_torso', category: 'Char_Armor_Torso' },
        { class: 'behr_rifle_ballistic_01', port: 'weapon_body_primary', category: 'Weapon_FPS_Rifle' },
      ] },
    }],
  }),
  'POST /v1/reference/resolve': s200({
    resolved: {
      grin_ballistic_helmet_01_black: { display_name: 'GRIN Ballistic Helmet', slug: 'grin-helmet', category: 'Char_Armor_Helmet', classification: 'FPS.Armor.Helmet', classification_label: 'Helmet', has_image: false },
      grin_ballistic_core_01_black: { display_name: 'GRIN Ballistic Core', slug: 'grin-core', category: 'Char_Armor_Torso', classification: 'FPS.Armor.Torso', classification_label: 'Torso', has_image: false },
      behr_rifle_ballistic_01: { display_name: 'BEHR P8-AR', slug: 'behr-p8-ar', category: 'Weapon_FPS_Rifle', classification: 'FPS.Weapon.Rifle', classification_label: 'Rifle', has_image: false },
    },
  }),
  // Owner viewing `/u/[handle]`: page.tsx short-circuits to the self path
  // before the public endpoints, so these keep the scenario deterministic.
  'GET /v1/public/TestPilot/summary': { status: 404, body: {} },
  'GET /v1/public/TestPilot/rsi-profile': { status: 404, body: {} },
  'GET /v1/public/TestPilot/rsi-orgs': { status: 200, body: { orgs: [] } },
};

/**
 * Measures the flat widget grid. It runs against `/u/[handle]` rather than
 * `/me`: the projection replaced `/me`, and this harness measures `.hud-tile`
 * geometry — a surface that now exists only on the public profile.
 */
test.describe('widget dashboard audit', () => {
  test.skip(!RUN, 'set AUDIT=1 to run the measurement harness');
  test.use({ viewport: { width: 1440, height: 900 } });

  test('measure every tile: clip / scroll / waste', async ({ page, request, context }) => {
    await context.addCookies([
      { name: 'ss-theme', value: 'dark', domain: 'localhost', path: '/' },
    ]);
    await resetScenario(request);
    await setScenario(request, scenarioFor('widget_audit', FIXTURES));
    await loginAs(page, { handle: 'TestPilot' });
    await page.goto('/u/TestPilot');
    await page.waitForLoadState('networkidle');
    await page.locator('section.hud-tile[data-widget-id]').first().waitFor();
    // Let client-side content auto-fit + compaction settle before measuring.
    await page.waitForTimeout(600);

    const report = await page.evaluate(() => {
      const doc = document.documentElement;
      const tiles = [...document.querySelectorAll('section.hud-tile[data-widget-id]')].map((el) => {
        const tile = el as HTMLElement;
        const body = tile.querySelector('.hud-tile__body') as HTMLElement | null;
        const empty = tile.querySelector('.hud-tile__empty');
        // REAL waste = body's available height minus the NATURAL height of
        // its content element (the widget's root child). Using the child's
        // own box avoids being fooled by a flex-stretched body whose
        // scrollHeight equals its clientHeight even when the visible content
        // is short. Header + padding is legitimate chrome, excluded.
        const availH = body ? body.clientHeight : 0;
        const child = body?.firstElementChild as HTMLElement | null;
        const contentH = child ? child.getBoundingClientRect().height : (body ? body.scrollHeight : 0);
        const scrolls = body ? body.scrollHeight > body.clientHeight + 1 : false;
        const wastePx = Math.max(0, Math.round(availH - contentH));
        return {
          id: tile.dataset.widgetId,
          size: tile.dataset.widgetSize,
          tileH: tile.clientHeight,
          bodyAvail: availH,
          contentH,
          wastePx,
          scrolls,
          empty: !!empty,
        };
      });
      return {
        pageOverflowX: doc.scrollWidth > doc.clientWidth,
        tileCount: tiles.length,
        wasteful: tiles.filter((t) => t.wastePx > 24).map((t) => `${t.id}(${t.wastePx})`),
        scrolling: tiles.filter((t) => t.scrolls).map((t) => t.id),
        tiles,
      };
    });

    // The report directory is not committed, so the harness creates it. It
    // used to ENOENT on a clean checkout — before this the run got all the way
    // through measuring and then threw on the write.
    mkdirSync(OUT_DIR, { recursive: true });
    writeFileSync(`${OUT_DIR}/widget-audit.json`, JSON.stringify(report, null, 2));
    await page.screenshot({ path: `${OUT_DIR}/widget-audit.png`, fullPage: true });

    // Console summary — the signal for the module work.
    const line = (t: (typeof report.tiles)[number]) =>
      `${(t.id ?? '?').padEnd(16)} ${String(t.size).padEnd(9)} tile=${String(t.tileH).padStart(4)} content=${String(t.contentH).padStart(4)} waste=${String(t.wastePx).padStart(4)} ${t.scrolls ? 'SCROLLS' : ''} ${t.empty ? 'EMPTY' : ''}`;
    // eslint-disable-next-line no-console
    console.log(
      `\n===== WIDGET AUDIT (${report.tileCount} tiles, pageOverflowX=${report.pageOverflowX}) =====\n` +
        report.tiles.map(line).join('\n') +
        '\n=====================================================\n',
    );
  });

  test('drill-down links navigate', async ({ page, request, context }) => {
    await context.addCookies([
      { name: 'ss-theme', value: 'dark', domain: 'localhost', path: '/' },
    ]);
    await resetScenario(request);
    await setScenario(request, scenarioFor('widget_audit_drill', FIXTURES));
    await loginAs(page, { handle: 'TestPilot' });
    await page.goto('/u/TestPilot');
    await page.waitForLoadState('networkidle');
    await page.locator('section.hud-tile[data-widget-id]').first().waitFor();
    // Past the auto-fit freeze window — the grid is now stable, so a normal
    // click can't miss a tile that is still moving under the cursor.
    await page.waitForTimeout(1800);

    // A drill-down link (in-tile "See more") navigates on a NORMAL click.
    const link = page.locator('section.hud-tile a', { hasText: 'See travel map' }).first();
    await expect(link).toHaveAttribute('href', '/me/travel');
    await link.scrollIntoViewIfNeeded();
    await link.click();
    await expect(page).toHaveURL(/\/me\/travel$/);

    // The sessions "See all" target used to 404 (no index page) — a 404 RSC
    // fetch bails the client nav back to the dashboard ("loads then
    // reverts"). The index page now exists and renders.
    await page.goto('/u/TestPilot/sessions');
    await expect(
      page.getByRole('heading', { name: /Sessions/i }),
    ).toBeVisible();
  });
});
