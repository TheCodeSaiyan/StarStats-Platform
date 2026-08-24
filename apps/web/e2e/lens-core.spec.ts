import { test, expect } from '@playwright/test';
import { loginAs, resetScenario, scenarioFor, setScenario } from './helpers/api-mock';

/**
 * The volume's centre follows the lens, and its rows go somewhere.
 *
 * Three faults this guards, all reported by the reader rather than caught by a
 * gate:
 *
 *   1. THE CENTRE WAS STATIC. `Holotable.jsx` says `const core = L ? L : OV` —
 *      the figure at the centre is the OPEN LENS's headline, and the lifetime
 *      anchor only when nothing is open. This screen reported logged flight
 *      time whichever lens you selected, which makes the ring look like
 *      decoration: you open Combat and the middle of the screen keeps counting
 *      hours flown.
 *   2. RAW IDENTIFIERS. "Ships you fly" listed `AEGS_Avenger_Stalker`; the
 *      location planes listed raw engine keys. The catalogue that resolves them
 *      was already loaded on the page for the hover cards.
 *   3. ROWS WENT NOWHERE. `MeterRow` takes an `onClick` and the server module
 *      building these planes never passed one — it cannot, a handler does not
 *      cross the RSC boundary. They are links now, which is better than a
 *      handler anyway: shareable, back-button correct, and working before
 *      hydration.
 */
const trace = { status: 200, body: { entries: [
  {ended_at:'2026-08-20T20:41:00Z',source_event_type:'resolve_spawn',started_at:'2026-08-20T20:31:00Z',event_count:12,system:'Stanton',planet:'Crusader',city:'Orison'},
  {ended_at:'2026-08-21T00:00:00Z',source_event_type:'resolve_spawn',started_at:'2026-08-20T21:44:00Z',event_count:8,system:'Stanton',planet:'ArcCorp',city:'Area18'},
] } };

const SCENARIO = {
      'GET /v1/me/location/trace': trace,
      'GET /v1/me/stats/combat': {
        status: 200,
        body: { kills: 52, deaths: 37, deaths_inferred: 9, hours: 8760 },
      },
      'GET /v1/me/stats/playtime': {
        status: 200,
        body: { total_playtime_secs: 418 * 3600, session_count: 184 },
      },
      'GET /v1/me/stats/locations': {
        status: 200,
        body: { hours: 8760, unique_locations: 214, top_locations: [] },
      },
      'GET /v1/me/summary': {
        status: 200,
        body: { total: 92481, by_type: [], first_event_at: null, last_event_at: null },
      },
      // Callout sources. Without these the callout field is empty and the
      // six-slot layout cannot be judged at all.
      'GET /v1/me/stats/lives': {
        status: 200,
        body: {
          total_lives: 41, deaths: 37, deaths_inferred: 9,
          lives_ended_by_crash: 2, sessions: 184,
          longest_life_secs: 4 * 3600 + 12 * 60, mean_life_secs: 22 * 60,
          deaths_per_session: 0.2, recent_lives: [], window: null,
        },
      },
      'GET /v1/me/stats/contracts': {
        status: 200,
        body: {
          // `total` gates the widget — 0 means "no data" and it renders
          // nothing. The headline buckets exclude Superseded rows; `runs`
          // includes them.
          total: 18, completed: 12, failed: 3, abandoned: 1,
          in_progress: 2, unknown: 0, completion_pct: 75,
          runs: [
            { name: 'Bounty: VHRT', state: 'completed' },
            { name: 'Bounty: VHRT', state: 'completed' },
            { name: 'Bounty: VHRT', state: 'failed' },
            { name: 'Cargo run', state: 'completed' },
          ],
        },
      },
      'GET /v1/me/stats/objectives': {
        status: 200,
        body: {
          completed: 84, failed: 9, unresolved: 7, no_outcome: 0,
          by_objective: [], lifetime: null, previous: null,
        },
      },
    } as Record<string, unknown>;

const COMMERCE = {
  'GET /v1/me/commerce/recent': { status: 200, body: { transactions: [
    { occurred_at: '2026-08-01T00:00:00Z', amount: 1200, kind: 'purchase', item: 'Ballista' } ] } },
  'GET /v1/me/stats/spend': { status: 200, body: {
    lifetime: { purchases: 84, total: 1200000 }, window: { purchases: 3, total: 4200 }, by_category: [] } },
};

const PLANES = {
  'GET /v1/me/stats/fleet': { status: 200, body: { ships: [
    { vehicle_class: 'AEGS_Avenger_Stalker', trip_count: 12 },
    { vehicle_class: 'ORIG_300i', trip_count: 5 } ] } },
  'GET /v1/me/stats/routes': { status: 200, body: { routes: [
    { destination: 'ArcCorp', count: 14 }, { destination: 'Crusader', count: 9 } ] } },
  'GET /v1/me/stats/docking': { status: 200, body: { total: 21, by_kind: [
    { key: 'ArcCorp', count: 14 } ] } },
};

async function coreOf(page: import('@playwright/test').Page) {
  return page.evaluate(() => {
    const c = document.querySelector('.hp-core');
    if (!c) return null;
    return {
      value: (c.querySelector('.n') as HTMLElement)?.textContent?.trim() ?? '',
      label: (c.querySelector('.u') as HTMLElement)?.textContent?.trim() ?? '',
    };
  });
}

test.beforeEach(async ({ request }) => {
  await resetScenario(request);
});

test('the centre of the volume changes with the open lens', async ({ page, request }) => {
  await setScenario(request, scenarioFor('lens-core', SCENARIO));
  await loginAs(page, { handle: 'TestPilot' });
  await page.goto('/me');
  await expect(page.locator('.hp-core')).toBeVisible();

  const overview = await coreOf(page);
  expect(overview?.label).toBe('Logged flight time');

  // Combat's headline is its own figure, not the lifetime anchor. Asserted as
  // a CHANGE rather than an exact string: the point is that the centre tracks
  // the lens, and pinning the wording would break on a copy edit.
  await page.locator('.hp-lens button', { hasText: 'Combat' }).click();
  await page.waitForTimeout(300);
  const combat = await coreOf(page);
  expect(combat?.label, 'the centre must not still read the lifetime anchor')
    .not.toBe(overview?.label);
  expect(combat?.value).not.toBe(overview?.value);

  // Returning to the overview restores the anchor.
  await page.locator('.hp-lens button', { hasText: 'All' }).click();
  await page.waitForTimeout(300);
  expect((await coreOf(page))?.label).toBe('Logged flight time');
});

test('a lens with no headline of its own keeps the anchor', async ({ page, request }) => {
  // The fallback is deliberate: a lens whose callouts a reader has switched
  // off shows the lifetime anchor rather than an invented figure or an empty
  // centre.
  await setScenario(request, scenarioFor('lens-core', SCENARIO));
  await loginAs(page, { handle: 'TestPilot' });
  await page.goto('/me');
  await expect(page.locator('.hp-core')).toBeVisible();
  await page.locator('.hp-lens button', { hasText: 'Loadout' }).click();
  await page.waitForTimeout(300);
  expect((await coreOf(page))?.label).toBe('Logged flight time');
});

test('ranked rows resolve their identifiers and link to the catalogue', async ({ page, request }) => {
  await setScenario(request, scenarioFor('lens-rows', { ...SCENARIO, ...PLANES }));
  await loginAs(page, { handle: 'TestPilot' });
  await page.goto('/me');
  await expect(page.locator('.hp-core')).toBeVisible();
  await page.locator('.hp-lens button', { hasText: 'Travel' }).click();
  await expect(page.locator('.hp-rw').first()).toBeVisible();

  const names = await page.locator('.hp-rw .nm').allTextContents();
  expect(names.length).toBeGreaterThan(0);
  // No raw engine identifier survives to the screen. `AEGS_Avenger_Stalker`
  // is what the API returns and what this used to render.
  for (const n of names) {
    expect(n, `raw identifier on screen: ${n}`).not.toMatch(/^[A-Z]{2,}_/);
    expect(n).not.toContain('_');
  }

  // And the row is a destination, not a dead end.
  const links = page.locator('.hp-rw a');
  expect(await links.count()).toBeGreaterThan(0);
  const href = await links.first().getAttribute('href');
  expect(href).toMatch(/^\/kb\//);
});


test('a lens whose figures are callouts is not reported as empty', async ({ page, request }) => {
  // Commerce's only enabled widget is a CALLOUT, and callouts do not appear in
  // a lens pane — the detail view hides the callout field. So the pane was
  // empty and drew "Nothing under Commerce in this window" over an account
  // that had commerce data. Measured before: 0 planes, 0 subs, 1 flatline.
  //
  // The lens pane now opens with its own figures, as `Holotable.jsx` does with
  // `L.subs`, and the flatline is suppressed when there are any.
  await setScenario(request, scenarioFor('lens-subs', { ...SCENARIO, ...COMMERCE }));
  await loginAs(page, { handle: 'TestPilot' });
  await page.goto('/me');
  await expect(page.locator('.hp-core')).toBeVisible();
  await page.locator('.hp-lens button', { hasText: 'Commerce' }).click();
  await page.waitForTimeout(400);

  const pane = page.locator('.hp-pane[data-pane="detail"]');
  await expect(pane.locator('.hp-subs > div')).not.toHaveCount(0);
  await expect(pane.locator('.hp-nosig')).toHaveCount(0);
});

test('an empty lens offers the range that would hold the data', async ({ page, request }) => {
  // `/me` defaults to 7 days while the figure at the centre is a LIFETIME
  // total, so a reader who has not played this week meets "nothing under
  // Combat" on an account with years behind it. That reads as lost data rather
  // than as a window, and the widest range was one click away and not offered.
  await setScenario(request, scenarioFor('lens-empty'));
  await loginAs(page, { handle: 'TestPilot' });
  await page.goto('/me');
  await expect(page.locator('.hp-core')).toBeVisible();
  await page.locator('.hp-lens button', { hasText: 'Commerce' }).click();
  await page.waitForTimeout(400);

  const flat = page.locator('.hp-pane[data-pane="detail"] .hp-nosig');
  await expect(flat).toHaveCount(1);
  const widen = flat.getByRole('link');
  await expect(widen).toHaveAttribute('href', /range=all/);
});

test('the trace names its own scale, so it reads as data', async ({ page, request }) => {
  // The trace is drawn from real daily counts normalised to its own peak,
  // which leaves it with no y-axis: a steady account draws a near-flat line
  // near the top and reads as decoration. The caption carries the scale.
  await setScenario(request, scenarioFor('lens-trace', {
    ...SCENARIO,
    'GET /v1/me/timeline': { status: 200, body: { days: 5, buckets: [
      { date: '2026-08-01', count: 4 }, { date: '2026-08-02', count: 90 },
      { date: '2026-08-03', count: 12 }, { date: '2026-08-04', count: 7 },
      { date: '2026-08-05', count: 39 } ] } },
  }));
  await loginAs(page, { handle: 'TestPilot' });
  await page.goto('/me');
  await expect(page.locator('.hp-core')).toBeVisible();
  await page.locator('.hp-lens button', { hasText: 'All' }).click();
  const cap = page.locator('.hp-grafcap');
  await expect(cap).toContainText('peak 90/day');
  await expect(cap).toContainText('152 events');

  // And the line follows the series rather than being a fixed shape.
  const spread = await page.locator('.hp-graf path').first().evaluate((p) => {
    const ys = [...(p.getAttribute('d') ?? '').matchAll(/L[\d.]+ ([\d.]+)/g)].map((m) => parseFloat(m[1]));
    return new Set(ys.map((y) => y.toFixed(1))).size;
  });
  expect(spread, 'a trace of varying counts must not be one flat line').toBeGreaterThan(2);
});


test('a lens shows every figure the widget holds, not just its headline', async ({ page, request }) => {
  // A callout carries ONE figure because that is all a leader line has room
  // for. The widget behind it holds three to five — `lives` knows total lives,
  // deaths and mean life — and none of that had anywhere to go in the
  // projection, so opening a lens showed strictly LESS than the flat widget it
  // replaced. The pane now renders the widget's own set.
  await setScenario(request, scenarioFor('lens-fullstats', SCENARIO));
  await loginAs(page, { handle: 'TestPilot' });
  await page.goto('/me');
  await expect(page.locator('.hp-core')).toBeVisible();
  await page.locator('.hp-lens button', { hasText: 'Combat' }).click();
  await page.waitForTimeout(400);

  const subs = page.locator('.hp-pane[data-pane="detail"] .hp-subs > div');
  // The lives callout alone contributes four: longest, mean, lives, deaths.
  expect(await subs.count()).toBeGreaterThanOrEqual(4);
  const text = await page.locator('.hp-pane[data-pane="detail"] .hp-subs').innerText();
  expect(text).toMatch(/Deaths/i);
  expect(text).toMatch(/Mean life/i);
});
