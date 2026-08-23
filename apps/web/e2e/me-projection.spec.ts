/**
 * The /me surface, in the projection.
 *
 * NOT a capture spec any more. This file began as scaffolding for the port —
 * a set of `goto` + `waitForTimeout` + `screenshot` cases whose only job was
 * producing images to judge, plus the fixtures they needed. Those 28 cases
 * asserted nothing, slept for half a second each, and are gone; what is left
 * are the assertions written alongside them, which are about behaviour and
 * outlive the port.
 */
import { test, expect } from '@playwright/test';
import { loginAs, resetScenario, scenarioFor, setScenario } from './helpers/api-mock';


/** A reader with real-shaped travel telemetry, so the ring has a map to draw. */
const trace = {
  status: 200,
  body: {
    entries: [
      { ended_at: '2026-08-20T20:41:00Z', source_event_type: 'resolve_spawn', started_at: '2026-08-20T20:31:00Z', event_count: 12, system: 'Stanton', planet: 'Crusader', city: 'Orison' },
      { ended_at: '2026-08-21T23:59:00Z', source_event_type: 'resolve_spawn', started_at: '2026-08-20T21:44:00Z', event_count: 8, system: 'Stanton', planet: 'ArcCorp', city: 'Area18' },
      { ended_at: '2026-08-21T23:59:00Z', source_event_type: 'resolve_spawn', started_at: '2026-08-20T22:51:00Z', event_count: 15, system: 'Stanton', planet: 'Crusader', city: 'Orison' },
      { ended_at: '2026-08-21T23:59:00Z', source_event_type: 'resolve_spawn', started_at: '2026-08-21T01:12:00Z', event_count: 6, system: 'Stanton', planet: 'Hurston', city: 'Lorville' },
      { ended_at: '2026-08-21T23:59:00Z', source_event_type: 'resolve_spawn', started_at: '2026-08-21T03:02:00Z', event_count: 9, system: 'Stanton', planet: 'microTech', city: 'New Babbage' },
      { ended_at: '2026-08-21T23:59:00Z', source_event_type: 'resolve_spawn', started_at: '2026-08-21T05:20:00Z', event_count: 4, system: 'Pyro', planet: 'Pyro I', city: null },
      { ended_at: '2026-08-21T23:59:00Z', source_event_type: 'resolve_spawn', started_at: '2026-08-21T07:44:00Z', event_count: 11, system: 'Stanton', planet: 'Crusader', city: 'Orison' },
    ],
  },
};

const consoleErrors: string[] = [];

test.beforeEach(async ({ page, request }) => {
  consoleErrors.length = 0;
  page.on('console', (m) => {
    if (m.type() !== 'error') return;
    // `resolveContractNames` calls `/api/contracts/resolve`, which is NOT on
    // the `/v1` prefix the mock server keys on, so it 599s in every e2e
    // scenario that renders contracts — pre-existing, and the widget already
    // degrades correctly (no link, plain text). Not a projection defect.
    if (m.text().includes('contracts resolve fetch failed')) return;
    consoleErrors.push(m.text());
  });
  page.on('pageerror', (e) => consoleErrors.push(`pageerror: ${e.message}`));
  await resetScenario(request);
  await setScenario(
    request,
    scenarioFor('projection-projection', {
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
    }),
  );
  await loginAs(page, { handle: 'StarStatsDemo' });
  await page.setViewportSize({ width: 1440, height: 900 });
});

test('picking a calibration repaints the beam in place', async ({ page }) => {
  // Same regression as on /settings: the persist action does not revalidate,
  // so rendering the server prop fired the recalibration event over a volume
  // that never changed colour. `data-cal` lives on the STAGE, not on <html> —
  // the projection deliberately keeps it off the root so the beam tokens
  // cannot reach the un-ported flat pages.
  await page.goto('/me');
  await expect(page.locator('.hp-core')).toBeVisible();
  await expect(page.locator('.hp-stage')).toHaveAttribute('data-cal', 'terra');

  await page.locator('.hp-cal button[aria-label="Pyro calibration"]').click();

  await expect(page.locator('.hp-stage')).toHaveAttribute('data-cal', 'pyro');
});

test('the page has exactly one h1, naming the reader', async ({ page }) => {
  // /me's crumb is a depth chain, not a page name, so its h1 is the handle —
  // exactly what the flat identity header carried — and is visually hidden.
  await page.goto('/me');
  await expect(page.locator('.hp-core')).toBeVisible();
  await expect(page.locator('h1')).toHaveCount(1);
  await expect(page.locator('h1')).toHaveText('@StarStatsDemo');
});

test('the layout editor offers every registered widget', async ({ page }) => {
  // Feature parity, end to end: the port dropped six widgets from the
  // catalogue entirely, so a reader who had enabled `hangar` or `loadout` on
  // the flat dashboard lost them with nothing to say so. The editor is where
  // that becomes visible, so it is asserted here as well as in the unit guard.
  await page.goto('/me');
  await expect(page.locator('.hp-core')).toBeVisible();
  await page.locator('.hp-stage').click({ position: { x: 5, y: 400 } });
  await page.keyboard.press('e');
  await expect(page.locator('.hp-layout')).toBeVisible();
  for (const name of [
    'Recent activity',
    'Records',
    'Orgs',
    'Hangar',
    'Player loadout',
    'Entities rollup',
  ]) {
    await expect(
      page.locator('.hp-layout .hp-el .nm', { hasText: name }),
    ).toHaveCount(1);
  }
});

test('no console errors on the overview', async ({ page }) => {
  await page.goto('/me');
  await expect(page.locator('.hp-core')).toBeVisible();
  await page.waitForTimeout(1200);
  // Printed rather than only asserted, so a failure names the actual message.
  if (consoleErrors.length) {
    console.log(`CONSOLE ERRORS:\n${consoleErrors.join('\n---\n')}`);
  }
  expect(consoleErrors).toEqual([]);
});

/**
 * `/me` against `Holotable.jsx` — the screen the whole system is designed
 * around, and one of the three COVERAGE marks as grounded.
 *
 * Three of its parts shipped missing, found by finally reading it: the ring
 * never switched to `bars` for an open lens, the detail pane had no `Trace`,
 * and the inspector has no event log. The first two are fixed and asserted
 * here; the third needs a per-location event query the API does not serve and
 * is still stated as a limit on screen rather than faked.
 */
test('an open lens switches the ring to its bar field', async ({
  page,
  request,
}) => {
  await setScenario(
    request,
    scenarioFor('me-ring-bars', {
      'GET /v1/me/timeline': {
        status: 200,
        body: {
          days: 182,
          buckets: Array.from({ length: 182 }, (_, i) => ({
            date: `2026-01-${String((i % 28) + 1).padStart(2, '0')}`,
            count: (i % 9) + 1,
          })),
        },
      },
    }),
  );
  await loginAs(page, { handle: 'StarStatsDemo' });
  await page.goto('/me');
  const stage = page.locator('.hp-stage');
  await expect(stage).toBeVisible();

  // Asserted through the SEGMENTS, because the bars are unclassed `<line>`
  // elements in the vendored `Ring` and adding a hook to it for a test would be
  // changing the design system to suit the suite. Segments present means
  // segment mode; segments absent with a lens open means it switched.
  // COUNT, not visibility: `.hp-seg` is an SVG `<path>` with no fill, so
  // Playwright's visibility check reports it hidden even when it is plainly
  // drawn — the same zero-box trap as the callout field and the BeamTip
  // trigger.
  const overviewSegments = await page.locator('.hp-seg').count();
  expect(overviewSegments).toBeGreaterThan(0);

  await page.locator('.hp-lens button', { hasText: 'Combat' }).click();
  await expect(page.locator('.hp-seg')).toHaveCount(0);
});

test('the lens detail carries a trace of real activity', async ({
  page,
  request,
}) => {
  await setScenario(
    request,
    scenarioFor('me-trace', {
      'GET /v1/me/timeline': {
        status: 200,
        body: {
          days: 182,
          buckets: Array.from({ length: 182 }, (_, i) => ({
            date: `2026-01-${String((i % 28) + 1).padStart(2, '0')}`,
            count: i,
          })),
        },
      },
    }),
  );
  await loginAs(page, { handle: 'StarStatsDemo' });
  await page.goto('/me');
  await page.locator('.hp-lens button', { hasText: 'Combat' }).click();
  const graf = page.locator('.hp-graf');
  await expect(graf).toBeVisible();
  // The caption names its own window, because it does NOT follow the range
  // control and a chart that silently did not would be misread.
  await expect(graf.locator('.hp-grafcap')).toHaveText(/last 26 weeks/i);
});

test('no timeline means no trace, not a flat line', async ({ page, request }) => {
  // A flat line reads as "no activity". The claim is "no data" — different
  // things, and the system's rule is that missing is absent, never zero.
  await setScenario(
    request,
    scenarioFor('me-trace-empty', {
      'GET /v1/me/timeline': { status: 200, body: { days: 182, buckets: [] } },
    }),
  );
  await loginAs(page, { handle: 'StarStatsDemo' });
  await page.goto('/me');
  await page.locator('.hp-lens button', { hasText: 'Combat' }).click();
  await expect(page.locator('.hp-graf')).toHaveCount(0);
});
