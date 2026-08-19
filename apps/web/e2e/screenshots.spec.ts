/**
 * Marketing screenshot generator — NOT a test.
 *
 * Drives the real web UI against the e2e MOCK backend (fake seeded data,
 * project-prefixed demo handles) and captures the pages the landing /
 * features surface use. No real account and no credential is involved.
 *
 * Fabricated DATA does not imply an unowned HANDLE. This spec used to say
 * "placeholder handle TestPilot" and treat the captures as scrub-free by
 * construction. Both "TestPilot" and "NovaPilot" turned out to be real RSI
 * citizens (checked 2026-08-17), so the published marketing images showed
 * real handles wearing invented stats, a supporter badge and a share
 * relationship — exactly the "one un-blurred handle" failure the scrub
 * checklist defends against, arrived at from the other direction.
 *
 * Every handle below is therefore `StarStatsDemo` / `SSDemo*`: project-prefixed
 * so nobody plausibly registers one later, and all five verified 404 at
 * robertsspaceindustries.com/en/citizens/<handle> on 2026-08-17. Keep the
 * prefix, and before publishing a capture containing a NEW handle, run the same
 * lookup (with a known-bad control, or a blanket-404 endpoint fools you) and
 * record it in apps/web/public/features/SCRUB-CHECKLIST.md.
 *
 * Skipped by default (it writes committed PNGs — not a CI test). Run:
 *   cd apps/web && CAPTURE=1 pnpm exec playwright test e2e/screenshots.spec.ts
 * Out:  apps/web/public/features/game-log-raw.png + marketing-*.png
 *
 * Regenerate any time the UI changes — that's the point of driving it from
 * fixtures rather than hand-scrubbing real captures.
 */
import { expect, test, type Page } from '@playwright/test';
import {
  loginAs,
  resetScenario,
  scenarioFor,
  setScenario,
} from './helpers/api-mock';

const OUT = 'public/features';
const DEMO_LABEL = 'DEMO';
const DEMO_DETAIL = 'Sample data — not a real player profile';

async function addDemoDisclosure(page: Page): Promise<void> {
  await page.evaluate(
    ({ detail, label }) => {
      const disclosure = document.createElement('div');
      disclosure.dataset['testid'] = 'marketing-demo-disclosure';
      disclosure.setAttribute('aria-label', `${label}: ${detail}`);
      disclosure.style.cssText = [
        'position:fixed', 'right:24px', 'top:72px', 'z-index:2147483647',
        'display:flex', 'align-items:center', 'gap:10px', 'padding:10px 14px',
        'border:1px solid #f0a52b', 'border-radius:4px',
        'background:rgba(10,10,14,.96)', 'box-shadow:0 8px 24px rgba(0,0,0,.45)',
        'color:#f3f0e8',
        'font:600 12px/1.2 ui-monospace,SFMono-Regular,Menlo,monospace',
        'letter-spacing:.02em', 'pointer-events:none',
      ].join(';');

      const badge = document.createElement('strong');
      badge.textContent = label;
      badge.style.cssText = [
        'padding:4px 7px', 'border-radius:2px', 'background:#f0a52b',
        'color:#0a0a0e', 'font-size:11px', 'letter-spacing:.12em',
      ].join(';');

      const copy = document.createElement('span');
      copy.textContent = detail;
      disclosure.append(badge, copy);
      document.body.append(disclosure);
    },
    { detail: DEMO_DETAIL, label: DEMO_LABEL },
  );

  const disclosure = page.getByTestId('marketing-demo-disclosure');
  await expect(disclosure).toContainText(DEMO_LABEL);
  await expect(disclosure).toContainText(DEMO_DETAIL);
}

const SYNTHETIC_LOG_LINES = [
  "<2026-07-17T16:54:29.778Z> [Notice] <ContextEstablisherStepStart> Starting CET 'CreateStreamingBubble' TraceID: demo-trace-01 [Team_Network][Replication][Loading]",
  "<2026-07-17T16:54:29.778Z> [Notice] <ContextEstablisherTaskFinished> establisher='Network' message='CET completed' taskname='CreateStreamingBubble' state='Finished' sessionId='00000000-0000-0000-0000-000000000001'",
  "<2026-07-17T16:54:29.779Z> [Notice] <ContextEstablisherStepStart> Starting CET 'UnstowPlayerVehicle' TraceID: demo-trace-02 [Team_Network][Replication][Loading]",
  "<2026-07-17T16:54:29.779Z> [Notice] <ContextEstablisherTaskFinished> establisher='Network' message='CET completed' taskname='UnstowPlayerVehicle' state='Finished' sessionId='00000000-0000-0000-0000-000000000001'",
  "<2026-07-17T16:54:29.780Z> [Notice] <ContextEstablisherStepStart> Starting CET 'UnstowPlayer' TraceID: demo-trace-03 [Team_Network][Replication][Loading]",
  "<2026-07-17T16:54:29.781Z> [Notice] <Context Establisher Unlocked> establisher='Game' taskname='CreatePlayerStreamingBubble' map='megapmap' gamerules='SC_Frontend'",
  "<2026-07-17T16:54:29.782Z> [Notice] <GameView> Bind Player: GameView enabled player entityId=1000000001 className='Player' [EntityStreaming][Protocol]",
  "<2026-07-17T16:54:29.783Z> [Notice] <ContextEstablisherTaskFinished> establisher='Game' message='CET completed' taskname='CreatePlayerStreamingBubble' state='Finished'",
  "<2026-07-17T16:54:29.784Z> [Notice] <Context Establisher Blocked> establisher='Game' taskname='BindAlwaysStreamedInEntities' sessionId='00000000-0000-0000-0000-000000000001'",
  "<2026-07-17T16:54:29.785Z> [Notice] <ContextEstablisherTaskFinished> establisher='Network' message='CET completed' [Team_Network][Replication][Loading]",
].join('\n\n');

/**
 * Rich fixtures for a fully-populated dashboard capture. Every widget's
 * "has data" gate is satisfied (traced from the tile/widget components).
 * Fabricated data, placeholder handle — nothing to scrub.
 */
const s200 = (body: unknown) => ({ status: 200, body });
/**
 * Anything the UI renders as RELATIVE time is derived from capture time rather
 * than pinned to a date. Absolute fixture timestamps aged badly: a month after
 * they were written, the hero capture read `sync 31d ago` in the warning red
 * and the share chips read `EXPIRES IN 2D`, so the marketing images advertised
 * a stale account and an expiring share. Byte-identical regeneration is worth
 * less than images that cannot rot.
 */
const NOW = Date.now();
const hoursAgo = (h: number) => new Date(NOW - h * 3_600_000).toISOString();
const daysAgo = (d: number) => new Date(NOW - d * 86_400_000).toISOString();
const daysAhead = (d: number) => new Date(NOW + d * 86_400_000).toISOString();

/** One dwell in the location trace. `corridors` folds consecutive stops
 *  into undirected A ⇄ B legs, so the ORDER of these entries is what
 *  produces corridors at all — a list of stops with no repeats yields
 *  no repeated leg and the tile stays empty. */
const traceStop = (
  city: string,
  planet: string,
  system: string,
  started_at: string,
  ended_at: string,
  event_count: number,
) => ({
  city,
  planet,
  system,
  shard: null,
  started_at,
  ended_at,
  event_count,
  source_event_type: 'location_update',
  resolved_location: null,
});

const richDashboard = {
  'GET /v1/me/stats/lives': s200({
    total_lives: 47,
    deaths: 39,
    sessions: 62,
    deaths_per_session: 0.6,
    lives_ended_by_crash: 4,
    longest_life_secs: 133200,
    mean_life_secs: 5400,
    recent_lives: [
      {
        death_inferred: false,
        death_zone: 'Yela',
        duration_secs: 5400,
        end_ts: '2026-07-14T22:10:00Z',
        ended_by: 'player_death',
        incap_count: 1,
        start_ts: '2026-07-14T20:40:00Z',
      },
    ],
  }),
  // `lifetime` twins are seeded on every endpoint that has one. Without
  // them the tiles render bare numbers: the comparison line is dropped
  // when the twin is absent (by design — a fabricated baseline is worse
  // than none), so a fixture missing it silently produces a screenshot
  // of the pre-comparison UI.
  'GET /v1/me/stats/fleet': s200({
    ships: [
      { vehicle_class: 'RSI_Constellation_Andromeda', trip_count: 84 },
      { vehicle_class: 'AEGS_Gladius', trip_count: 51 },
      { vehicle_class: 'DRAK_Cutlass_Black', trip_count: 33 },
    ],
    lifetime: { total_trips: 1284, ships_flown: 11 },
    previous: { total_trips: 141, ships_flown: 3 },
  }),
  'GET /v1/me/stats/docking': s200({
    total_stows: 120,
    by_kind: { hangar: 80, pad: 30, other: 10 },
    by_size: { small: 40, medium: 45, large: 25, xl: 10, unknown: 0 },
    lifetime: { total_stows: 947 },
    previous: { total_stows: 104 },
  }),
  'GET /v1/me/stats/spend': s200({
    total_auec: 184_500,
    purchases: 23,
    top_shop: 'SCShop_Aparelli_NewBabbage',
    lifetime: { total_auec: 2_410_000, purchases: 318 },
    previous: { total_auec: 121_800, purchases: 17 },
  }),
  'GET /v1/me/stats/objectives': s200({
    completed: 180,
    failed: 46,
    unresolved: 52,
    no_outcome: 41,
    total: 319,
    completion_pct: 65,
    lifetime: {
      completed: 1_204,
      failed: 233,
      unresolved: 310,
      no_outcome: 288,
      total: 2_035,
      completion_pct: 68,
    },
    previous: {
      completed: 156,
      failed: 39,
      unresolved: 44,
      no_outcome: 37,
      total: 276,
      completion_pct: 65,
    },
  }),
  'GET /v1/me/stats/routes': s200({
    routes: [
      { destination: 'microTech', count: 62 },
      { destination: 'Crusader', count: 48 },
      { destination: 'ArcCorp', count: 29 },
      { destination: 'Hurston', count: 21 },
    ],
    lifetime: { total_trips: 1284, destinations: 34 },
    previous: { total_trips: 133, destinations: 6 },
  }),
  // Corridors AND journey both read the location trace. Leaving it
  // unseeded is why both tiles rendered "No Telemetry Signal Found" in
  // the previous capture — an empty state is not what a marketing shot
  // should be showing.
  'GET /v1/me/location/trace': s200({
    hours: 168,
    entries: [
      traceStop('Orison', 'Crusader', 'stanton', '2026-07-28T18:04:00Z', '2026-07-28T19:26:00Z', 62),
      traceStop('New Babbage', 'microTech', 'stanton', '2026-07-28T19:41:00Z', '2026-07-28T21:12:00Z', 78),
      traceStop('Area18', 'ArcCorp', 'stanton', '2026-07-29T17:22:00Z', '2026-07-29T18:44:00Z', 54),
      traceStop('Orison', 'Crusader', 'stanton', '2026-07-29T19:03:00Z', '2026-07-29T20:38:00Z', 71),
      traceStop('New Babbage', 'microTech', 'stanton', '2026-07-30T18:15:00Z', '2026-07-30T19:52:00Z', 66),
      traceStop('Lorville', 'Hurston', 'stanton', '2026-07-30T20:10:00Z', '2026-07-30T21:31:00Z', 49),
      traceStop('Orison', 'Crusader', 'stanton', '2026-07-31T17:48:00Z', '2026-07-31T19:20:00Z', 58),
      traceStop('New Babbage', 'microTech', 'stanton', '2026-07-31T19:35:00Z', '2026-07-31T21:04:00Z', 73),
    ],
  }),
  'GET /v1/me/stats/contracts': s200({
    total: 64,
    completed: 41,
    failed: 9,
    abandoned: 7,
    withdrawn: 3,
    in_progress: 2,
    unknown: 2,
    completion_pct: 64,
    runs: [],
  }),
  'GET /v1/me/stats/combat': s200({
    hours: 720,
    kills: 128,
    deaths: 39,
    top_weapons: [
      { value: 'behr_ballistic_rifle', count: 44 },
      { value: 'gallenson_gatling', count: 31 },
    ],
    deaths_by_zone: [
      { value: 'Yela', count: 12 },
      { value: 'Daymar', count: 9 },
    ],
  }),
  'GET /v1/me/stats/playtime': s200({
    hours: 418,
    session_count: 62,
    total_playtime_secs: 418 * 3600,
  }),
  'GET /v1/me/stats/locations': s200({
    hours: 720,
    unique_locations: 67,
    top_locations: [
      { value: 'Orison', count: 88 },
      { value: 'Area18', count: 64 },
      { value: 'New Babbage', count: 51 },
    ],
  }),
  'GET /v1/me/stats/biggest-trade': s200({ quantity: 384, item: 'Titanium' }),
  'GET /v1/me/stats/records': s200({
    longest_session_secs: 21600,
    busiest_session_events: 512,
    longest_survival_streak_secs: 187200,
    deadliest_session_deaths: 7,
  }),
  'GET /v1/me/commerce/recent': s200({
    transactions: [
      {
        kind: 'shop',
        status: 'confirmed',
        item: 'Ballistic Gatling',
        quantity: 1,
        raw_request: 'buy',
        started_at: '2026-07-15T18:00:00Z',
        confirmed_at: '2026-07-15T18:00:05Z',
        shop_id: 'shop_dumpers',
      },
      {
        kind: 'commodity_buy',
        status: 'confirmed',
        item: 'Laranite',
        quantity: 96,
        raw_request: 'buy',
        started_at: '2026-07-15T12:00:00Z',
      },
      {
        kind: 'commodity_sell',
        status: 'pending',
        item: 'Laranite',
        quantity: 96,
        raw_request: 'sell',
        started_at: '2026-07-16T09:00:00Z',
      },
    ],
  }),
  // Seeded so the Flight facts widget renders content. Unseeded it renders its
  // "Facts need 8 sessions to say anything meaningful — you have 0" empty
  // state, which then ships inside the hero marketing capture.
  'GET /v1/me/facts': s200({
    enough_history: true,
    sessions_considered: 62,
    sessions_required: 8,
    facts: [
      {
        id: 'fleet-favourite',
        scope: 'lifetime',
        headline: 'The Constellation Andromeda is half of your flight time',
        detail: '84 of 168 trips across 62 sessions',
        evidence: { value: 84, baseline: 168, sample_size: 62, unit: 'count' },
        provenance: 'vehicle control flow events',
      },
      {
        id: 'long-hauler',
        scope: 'lifetime',
        headline: 'Your sessions run longer than they used to',
        detail: 'Mean 2h 41m over the last 7 days against 1h 52m lifetime',
        evidence: { value: 161, baseline: 112, sample_size: 62, unit: 'minutes' },
        provenance: 'session durations',
      },
    ],
  }),  'GET /v1/me/location/current': s200({
    location: {
      city: 'Orison',
      planet: 'Crusader',
      system: 'Stanton',
      shard: 'pub_euw1b',
      last_seen_at: hoursAgo(2),
      entered_at: hoursAgo(5),
      entered_at_is_lower_bound: false,
      source_event_type: 'location_inventory_requested',
    },
  }),
  'GET /v1/me/hangar': s200({
    captured_at: hoursAgo(3),
    ships: [
      { name: 'Constellation Andromeda', manufacturer: 'RSI', kind: 'ship' },
      { name: 'Gladius', manufacturer: 'Aegis Dynamics', kind: 'ship' },
      { name: 'Cutlass Black', manufacturer: 'Drake Interplanetary', kind: 'ship' },
    ],
  }),
  'GET /v1/me/profile': s200({
    badges: [],
    bio: 'Synthetic demo pilot.',
    captured_at: daysAgo(1),
    display_name: 'StarStatsDemo',
    enlistment_date: '2015-04-13',
    location: 'Stanton',
    primary_org_summary: 'Demo Flight [Member]',
  }),
  'GET /v1/me/supporter': s200({
    state: 'active',
    current_tier_key: 'generous',
    name_plate: 'Ace',
    became_supporter_at: '2026-05-31T22:00:00Z',
    last_payment_at: '2026-05-31T22:00:00Z',
    grace_until: null,
    cancelled_at: null,
  }),
  'GET /v1/users/StarStatsDemo/sessions': s200({
    sessions: [
      { id: 'sess-1', started_at: hoursAgo(6), ended_at: hoursAgo(3.5), event_count: 142 },
      { id: 'sess-2', started_at: hoursAgo(34), ended_at: hoursAgo(30.75), event_count: 97 },
      { id: 'sess-3', started_at: hoursAgo(58), ended_at: hoursAgo(55.33), event_count: 63 },
    ],
  }),
  'GET /v1/users/me/profile-layout': s200({ layout: null }),
  // Travel + Combat&Missions widgets both read this breakdown:
  // Travel needs quantum_target_selected; Combat sums death/vehicle/mission.
  'GET /v1/me/metrics/event-types': s200({
    types: [
      { event_type: 'login', count: 62 },
      { event_type: 'quantum_target_selected', count: 214 },
      { event_type: 'location_changed', count: 168 },
      { event_type: 'player_death', count: 34 },
      { event_type: 'actor_death', count: 5 },
      { event_type: 'vehicle_destruction', count: 22 },
      { event_type: 'mission_start', count: 48 },
      { event_type: 'mission_end', count: 41 },
      { event_type: 'commodity_buy_request', count: 73 },
    ],
  }),
  'GET /v1/me/stats/travel': s200({
    hours: 2160,
    quantum_jumps: 1840,
    planets_visited: [
      { value: 'Crusader', count: 312 },
      { value: 'Hurston', count: 246 },
      { value: 'microTech', count: 198 },
      { value: 'ArcCorp', count: 144 },
    ],
    top_destinations: [
      { value: 'Port Tressler', count: 96 },
      { value: 'Everus Harbor', count: 82 },
      { value: 'CRU-L1', count: 64 },
    ],
  }),
  'GET /v1/me/location/breakdown': s200({
    hours: 168,
    entries: [
      { city: 'Orison', planet: 'Crusader', system: 'Stanton', dwell_seconds: 34200, visit_count: 8 },
      { city: 'New Babbage', planet: 'microTech', system: 'Stanton', dwell_seconds: 28800, visit_count: 6 },
      { city: 'Lorville', planet: 'Hurston', system: 'Stanton', dwell_seconds: 21600, visit_count: 5 },
      { city: 'Area18', planet: 'ArcCorp', system: 'Stanton', dwell_seconds: 18000, visit_count: 4 },
    ],
  }),
  'GET /v1/me/stats/loadout-activity': s200({
    equips: 73,
    stores: 28,
    top_items: [
      { item_class: 'behr_rifle_ballistic_01', count: 18 },
      { item_class: 'grin_ballistic_core_01_black', count: 14 },
      { item_class: 'medpen_01', count: 11 },
    ],
  }),
  'GET /v1/public/u/StarStatsDemo/orgs': s200({
    captured_at: daysAgo(1),
    orgs: [
      { name: 'Demo Flight', sid: 'DEMO', rank: 'Member', member_count: 48, logo_url: null, is_main: true },
      { name: 'Stanton Explorers', sid: 'STX', rank: 'Pathfinder', member_count: 73, logo_url: null, is_main: false },
    ],
  }),
};

// --- Loadout: one rich burst_summary + a resolve map so the paperdoll fills.
const richLoadout = {
  'GET /v1/me/events': s200({
    next_after: null,
    events: [
      {
        event_timestamp: '2026-07-18T20:14:03Z',
        event_type: 'burst_summary',
        hidden_at: null,
        log_source: 'game.log',
        seq: 91422,
        source_offset: 5540123,
        resolved_location: null,
        payload: {
          kind: 'loadout_restore',
          items: [
            { class: 'grin_ballistic_helmet_01_black', port: 'armor_helmet', category: 'Char_Armor_Helmet' },
            { class: 'grin_ballistic_core_01_black', port: 'armor_torso', category: 'Char_Armor_Torso' },
            { class: 'grin_ballistic_arms_01_black', port: 'armor_arms', category: 'Char_Armor_Arms' },
            { class: 'grin_ballistic_legs_01_black', port: 'armor_legs', category: 'Char_Armor_Legs' },
            { class: 'pmbs_backpack_medium_01_green', port: 'backpack', category: 'Char_Armor_Backpack' },
            { class: 'behr_rifle_ballistic_01', port: 'weapon_body_primary', category: 'Weapon_FPS_Rifle' },
            { class: 'behr_rifle_ballistic_01_mag_01', port: 'magazine_well', category: 'Weapon_FPS_Magazine' },
            { class: 'apar_smg_ballistic_01', port: 'weapon_body_sidearm', category: 'Weapon_FPS_SMG' },
            { class: 'frag_grenade_01', port: 'grenade_attach_left', category: 'Char_Consumable_Grenade' },
            { class: 'medpen_01', port: 'utility_attach_01', category: 'Char_Consumable_Medical' },
            { class: 'cryapt_multitool_01', port: 'utility_attach_02', category: 'Char_Tool_Multitool' },
          ],
        },
      },
    ],
  }),
  'POST /v1/reference/resolve': s200({
    resolved: {
      grin_ballistic_helmet_01_black: { display_name: 'GRIN Ballistic Helmet (Black)', slug: 'grin-ballistic-helmet', category: 'Char_Armor_Helmet', classification: 'FPS.Armor.Helmet', classification_label: 'Helmet', has_image: false },
      grin_ballistic_core_01_black: { display_name: 'GRIN Ballistic Core (Black)', slug: 'grin-ballistic-core', category: 'Char_Armor_Torso', classification: 'FPS.Armor.Torso', classification_label: 'Torso', has_image: false },
      grin_ballistic_arms_01_black: { display_name: 'GRIN Ballistic Arms (Black)', slug: 'grin-ballistic-arms', category: 'Char_Armor_Arms', classification: 'FPS.Armor.Arms', classification_label: 'Arms', has_image: false },
      grin_ballistic_legs_01_black: { display_name: 'GRIN Ballistic Legs (Black)', slug: 'grin-ballistic-legs', category: 'Char_Armor_Legs', classification: 'FPS.Armor.Legs', classification_label: 'Legs', has_image: false },
      pmbs_backpack_medium_01_green: { display_name: 'PMBS Medium Backpack (Green)', slug: 'pmbs-medium-backpack', category: 'Char_Armor_Backpack', classification: 'FPS.Armor.Backpack', classification_label: 'Backpack', has_image: false },
      behr_rifle_ballistic_01: { display_name: 'BEHR P8-AR Rifle', slug: 'behr-p8-ar', category: 'Weapon_FPS_Rifle', classification: 'FPS.Weapon.Rifle', classification_label: 'Rifle', has_image: false },
      behr_rifle_ballistic_01_mag_01: { display_name: 'P8-AR Magazine', slug: 'behr-p8-ar-mag', category: 'Weapon_FPS_Magazine', classification: 'FPS.WeaponAttachment.Magazine', classification_label: 'Magazine', has_image: false },
      apar_smg_ballistic_01: { display_name: 'Apar P6 SMG', slug: 'apar-p6', category: 'Weapon_FPS_SMG', classification: 'FPS.Weapon.SMG', classification_label: 'SMG', has_image: false },
      frag_grenade_01: { display_name: 'FS-9 Frag Grenade', slug: 'fs-9-frag', category: 'Char_Consumable_Grenade', classification: null, classification_label: null, has_image: false },
      medpen_01: { display_name: 'MedPen', slug: 'medpen', category: 'Char_Consumable_Medical', classification: 'FPS.Consumable.Medical', classification_label: 'Medical', has_image: false },
      cryapt_multitool_01: { display_name: 'Cryapt Multi-Tool', slug: 'cryapt-multitool', category: 'Char_Tool_Multitool', classification: null, classification_label: null, has_image: false },
    },
  }),
};

// --- Sharing: public profile with outbound/inbound shares + view stats.
const richSharing = {
  'GET /v1/me/visibility': s200({ public: true, listing_opt_out: false }),
  'GET /v1/me/shares': s200({
    shares: [
      { recipient_handle: 'SSDemoWingman', expires_at: daysAhead(30), last_viewed_at: hoursAgo(50), note: 'Wingman — combat stats', scope: { kind: 'aggregates' }, view_count: 14 },
      { recipient_handle: 'SSDemoAstra', expires_at: null, last_viewed_at: null, note: null, scope: null, view_count: 0 },
    ],
    org_shares: [{ org_slug: 'ssdemo-wing' }],
  }),
  'GET /v1/me/shared-with-me': s200({
    shared_with_me: [
      { owner_handle: 'SSDemoOrgLead', expires_at: null, note: 'Full manifest — org lead', scope: null },
      { owner_handle: 'SSDemoMiner', expires_at: daysAhead(90), note: 'Sharing my mining runs', scope: { kind: 'tabs', tabs: ['commerce', 'travel'] } },
    ],
  }),
  'GET /v1/orgs': s200({
    orgs: [
      { id: 'org_a', name: 'StarStats Demo Wing', slug: 'ssdemo-wing', owner_user_id: 'usr_1', created_at: '2026-01-04T10:00:00Z' },
      { id: 'org_b', name: 'Deep Black', slug: 'deep-black', owner_user_id: 'usr_2', created_at: '2026-02-11T10:00:00Z' },
    ],
  }),
  'GET /v1/me/profile-views': s200({
    totals: { all_time: 1284, last_7d: 39, last_30d: 173, by_source_30d: { direct: 96, discover: 51, shared: 22, other: 4 } },
    days: [
      { day: '2026-07-19', total: 11, by_source: { direct: 7, discover: 3, shared: 1 } },
      { day: '2026-07-18', total: 9, by_source: { direct: 5, discover: 3, shared: 1 } },
      { day: '2026-07-17', total: 14, by_source: { direct: 8, discover: 4, shared: 2 } },
      { day: '2026-07-16', total: 6, by_source: { direct: 4, discover: 2 } },
      { day: '2026-07-15', total: 12, by_source: { direct: 7, discover: 3, shared: 1, other: 1 } },
      { day: '2026-07-14', total: 8, by_source: { direct: 5, discover: 3 } },
      { day: '2026-07-13', total: 5, by_source: { direct: 3, discover: 1, shared: 1 } },
    ],
  }),
};

// Marketing viewport: a clean 16:10 desktop frame.
test.use({ viewport: { width: 1440, height: 900 } });

test.beforeEach(async ({ request, context }) => {
  // Generator, not a test: skipped unless explicitly capturing, so CI's
  // e2e run never regenerates (and dirties) the committed PNGs.
  test.skip(process.env['CAPTURE'] !== '1', 'set CAPTURE=1 to generate screenshots');
  await resetScenario(request);
  // Pin dark theme for a consistent, on-brand (cockpit/HUD) look.
  await context.addCookies([
    { name: 'ss-theme', value: 'dark', domain: 'localhost', path: '/' },
  ]);
  // Next renders its dev-mode indicator into a `nextjs-portal` custom
  // element. The harness runs `next dev` (see playwright.config.ts), so
  // without this the Next.js dev badge is baked into every marketing
  // PNG — it was visible in the captures committed before this line.
  await context.addInitScript(() => {
    const css = 'nextjs-portal{display:none!important}';
    const apply = () => {
      const s = document.createElement('style');
      s.textContent = css;
      document.head?.appendChild(s);
    };
    if (document.head) apply();
    else document.addEventListener('DOMContentLoaded', apply);
  });
});

test('marketing_game_log', async ({ page }) => {
  await page.setViewportSize({ width: 1133, height: 746 });
  await page.setContent(`
    <!doctype html>
    <html lang="en">
      <head>
        <meta charset="utf-8" />
        <title>Game.log — synthetic sample</title>
        <style>
          * { box-sizing: border-box; }
          html, body { margin: 0; min-height: 100%; background: #17191d; color: #f4f4f4; }
          body { font-family: Consolas, "Cascadia Mono", monospace; padding-top: 132px; }
          header { position: fixed; inset: 0 0 auto 0; height: 44px; display: flex;
            align-items: center; gap: 28px; padding: 0 16px; background: #202328;
            border-bottom: 1px solid #30343a; font: 14px/1 "Segoe UI", sans-serif; }
          header strong { margin-left: auto; color: #f0a52b; font: 600 12px Consolas, monospace; }
          pre { margin: 0; padding: 0 16px 24px; white-space: pre-wrap;
            font: 14px/1.35 Consolas, "Cascadia Mono", monospace; }
        </style>
      </head>
      <body>
        <header><span>File</span><span>Edit</span><span>View</span><strong>Game.log — synthetic sample</strong></header>
        <pre></pre>
      </body>
    </html>
  `);
  await page.locator('pre').evaluate((element, value) => {
    element.textContent = value;
  }, SYNTHETIC_LOG_LINES);
  await addDemoDisclosure(page);
  await page.screenshot({ path: `${OUT}/game-log-raw.png` });
});

test('marketing_dashboard', async ({ page, request }) => {
  await setScenario(request, scenarioFor('shot_dashboard', richDashboard));
  await loginAs(page, { handle: 'StarStatsDemo' });
  await page.goto('/me');
  await page.waitForLoadState('networkidle');
  await addDemoDisclosure(page);
  await page.screenshot({ path: `${OUT}/marketing-dashboard.png`, fullPage: true });
});

test('marketing_loadout', async ({ page, request }) => {
  await setScenario(request, scenarioFor('shot_loadout', richLoadout));
  await loginAs(page, { handle: 'StarStatsDemo' });
  await page.goto('/me/loadout');
  await page.waitForLoadState('networkidle');
  await addDemoDisclosure(page);
  await page.screenshot({ path: `${OUT}/marketing-loadout.png`, fullPage: true });
});

test('marketing_sharing', async ({ page, request }) => {
  await setScenario(request, scenarioFor('shot_sharing', richSharing));
  await loginAs(page, { handle: 'StarStatsDemo' });
  await page.goto('/sharing');
  await page.waitForLoadState('networkidle');
  await addDemoDisclosure(page);
  await page.screenshot({ path: `${OUT}/marketing-sharing.png`, fullPage: true });
});
