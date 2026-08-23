/**
 * The travel surface, in the projection.
 *
 * NOT a capture spec any more. This file began as scaffolding for the port —
 * a set of `goto` + `waitForTimeout` + `screenshot` cases whose only job was
 * producing images to judge, plus the fixtures they needed. Those 28 cases
 * asserted nothing, slept for half a second each, and are gone; what is left
 * are the assertions written alongside them, which are about behaviour and
 * outlive the port.
 */
import { test, expect, type Page } from '@playwright/test';
import { loginAs, resetScenario, scenarioFor, setScenario } from './helpers/api-mock';

const consoleErrors: string[] = [];

const trace = {
  status: 200,
  body: {
    entries: [
      { ended_at: '2026-08-20T20:41:00Z', source_event_type: 'resolve_spawn', started_at: '2026-08-20T20:31:00Z', event_count: 12, system: 'Stanton', planet: 'Crusader', city: 'Orison' },
      { ended_at: '2026-08-21T00:00:00Z', source_event_type: 'resolve_spawn', started_at: '2026-08-20T21:44:00Z', event_count: 8, system: 'Stanton', planet: 'ArcCorp', city: 'Area18' },
      { ended_at: '2026-08-21T02:00:00Z', source_event_type: 'resolve_spawn', started_at: '2026-08-21T01:12:00Z', event_count: 6, system: 'Stanton', planet: 'Hurston', city: 'Lorville' },
      { ended_at: '2026-08-21T06:00:00Z', source_event_type: 'resolve_spawn', started_at: '2026-08-21T05:20:00Z', event_count: 4, system: 'Pyro', planet: 'Pyro I', city: null },
    ],
  },
};

const FIXTURES = {
  'GET /v1/me/location/trace': trace,
  'GET /v1/me/stats/routes': {
    status: 200,
    body: {
      routes: [
        { destination: 'Orison', count: 14 },
        { destination: 'Area18', count: 9 },
        { destination: 'Lorville', count: 5 },
        { destination: 'New Babbage', count: 3 },
      ],
    },
  },
  'GET /v1/me/stats/travel': {
    status: 200,
    body: { quantum_jumps: 31, planets_visited: ['Crusader', 'Hurston'] },
  },
};

test.beforeEach(async ({ page, request }) => {
  consoleErrors.length = 0;
  page.on('console', (m) => {
    if (m.type() === 'error') consoleErrors.push(m.text());
  });
  page.on('pageerror', (e) => consoleErrors.push(`pageerror: ${e.message}`));
  await resetScenario(request);
  await setScenario(request, scenarioFor('travel-projection', FIXTURES));
  await loginAs(page, { handle: 'StarStatsDemo' });
  await page.setViewportSize({ width: 1440, height: 900 });
});

async function openGroup(page: Page, name: string): Promise<void> {
  await page.locator('.hp-lens button', { hasText: name }).click();
}

test('the range control drives the URL, not client state', async ({ page }) => {
  await page.goto('/me/travel');
  await expect(page.locator('.hp-settings')).toBeVisible();
  await page.locator('.hp-rng a', { hasText: '30d' }).click();
  await expect(page).toHaveURL(/\/me\/travel\?range=30d/);
});

test('the page has exactly one h1, naming the page', async ({ page }) => {
  await page.goto('/me/travel');
  await expect(page.locator('h1')).toHaveCount(1);
  await expect(page.locator('h1')).toHaveText('Travel');
});

test('no console errors across every group', async ({ page }) => {
  await page.goto('/me/travel');
  await expect(page.locator('.hp-settings')).toBeVisible();
  for (const g of ['Trail', 'Routes']) {
    await openGroup(page, g);
    await page.waitForTimeout(250);
  }
  await page.waitForTimeout(900);
  if (consoleErrors.length) {
    console.log(`CONSOLE ERRORS:\n${consoleErrors.join('\n---\n')}`);
  }
  expect(consoleErrors).toEqual([]);
});

test('a lens is never offered with nothing behind it', async ({
  page,
  request,
}) => {
  // Reported from a real look at the page: the Trail lens was there and its
  // volume was empty. Travel declares its groups statically and builds its
  // sections from data, so a reader with no stops got a control that lit up
  // and showed nothing — the same shape as the Emitter's blank surface.
  //
  // `PaneSurface` now filters the rail to groups that actually have sections,
  // so this asserts the general rule on the surface that surfaced it: every
  // lens in the rail, when selected, shows at least one pane.
  await setScenario(
    request,
    scenarioFor('travel-empty-trail', {
      'GET /v1/me/stats/routes': { status: 200, body: { routes: [] } },
      'GET /v1/me/location/trace': { status: 200, body: { entries: [] } },
    }),
  );
  await loginAs(page, { handle: 'StarStatsDemo' });
  await page.goto('/me/travel');
  await expect(page.locator('.hp-settings')).toBeVisible();

  const lenses = page.locator('.hp-lens button');
  const count = await lenses.count();
  for (let i = 0; i < count; i += 1) {
    const label = (await lenses.nth(i).textContent()) ?? `lens ${i}`;
    await lenses.nth(i).click();
    await expect(page.locator('.hp-phd h2').first(), label).toBeVisible();
  }
});

test('travel browses locations by taxonomy level', async ({ page, request }) => {
  // `Journey.jsx` browses through a category strip — Systems / Planets /
  // Cities / Sites, each with a count — where this page had one fixed level,
  // so a reader could ask "which systems have I visited" and nothing else.
  //
  // Nothing is fetched for it: `DistinctStop` already carries system, planet
  // and city, and the stop itself is the site.
  // A trace fixture is REQUIRED, not incidental: `PaneSurface` no longer
  // offers a lens with no sections behind it, so without stops the Trail lens
  // correctly does not exist and there is nothing to click.
  await setScenario(
    request,
    scenarioFor('travel-taxonomy', {
      'GET /v1/me/location/trace': {
        status: 200,
        body: {
          hours: 168,
          entries: [
            {
              started_at: '2026-08-01T10:00:00Z',
              ended_at: '2026-08-01T12:00:00Z',
              event_count: 40,
              source_event_type: 'location_change',
              system: 'Stanton',
              planet: 'Crusader',
              city: 'Orison',
            },
            {
              started_at: '2026-08-02T10:00:00Z',
              ended_at: '2026-08-02T11:00:00Z',
              event_count: 22,
              source_event_type: 'location_change',
              system: 'Stanton',
              planet: 'ArcCorp',
              city: 'Area18',
            },
            {
              started_at: '2026-08-03T10:00:00Z',
              ended_at: '2026-08-03T11:30:00Z',
              event_count: 31,
              source_event_type: 'location_change',
              system: 'Pyro',
              planet: 'Pyro I',
              city: null,
            },
          ],
        },
      },
    }),
  );
  await loginAs(page, { handle: 'StarStatsDemo' });
  await page.goto('/me/travel');
  await expect(page.locator('.hp-settings')).toBeVisible();
  await page.locator('.hp-lens button', { hasText: 'Trail' }).click();

  const strip = page.locator('.hp-catstrip', { hasText: 'Systems' });
  await expect(strip).toHaveCount(1);
  await expect(strip.locator('.hp-catchip')).toHaveCount(4);
  // Exactly one level is current, and it is the default.
  await expect(strip.locator('[aria-current="page"]')).toHaveText(/Systems/);

  // The level is URL state, not client state — shareable and back-button
  // correct, like the range control beside it.
  await strip.locator('.hp-catchip', { hasText: 'Cities' }).click();
  await expect(page).toHaveURL(/level=city/);
  await expect(
    page.locator('.hp-catstrip [aria-current="page"]'),
  ).toHaveText(/Cities/);
});

test('a place opens its own record, and the level list descends into it', async ({
  page,
}) => {
  // `Journey.jsx` runs `CatalogueLayout`'s three-part shell and the port
  // shipped two of them: the level tabs and the ranked list. Selecting a row
  // did nothing, so the screen could tell you that you had been to a city and
  // nothing whatsoever about the city. This is the missing third part.
  await page.goto('/me/travel?level=city');
  await expect(page.locator('.hp-settings')).toBeVisible();
  await openGroup(page, 'Trail');

  const row = page.locator('.hp-rw a').first();
  const name = (await row.innerText()).trim();
  await row.click();

  // URL state, not client state: a place has to be shareable and the back
  // button has to climb out of it.
  await expect(page).toHaveURL(/place=/);

  const detail = page.locator('.hp-journeydetail');
  await expect(detail).toBeVisible();
  // The figures, and the record panes beside them.
  await expect(detail.locator('.hp-subs > div')).not.toHaveCount(0);
  await expect(
    detail.locator('.hp-plane', { hasText: 'Scope record' }),
  ).toBeVisible();
  await expect(
    detail.locator('.hp-plane', { hasText: 'Arrivals' }),
  ).toBeVisible();
  // The pane is headed by the place itself, not by the generic list title.
  // Scoped to the pane that CONTAINS the detail — the trail group stacks
  // several panes and `.hp-phd h2` first matches "Location trail".
  const ownPane = page.locator('.hp-pane', {
    has: page.locator('.hp-journeydetail'),
  });
  await expect(ownPane.locator('.hp-phd h2')).toContainText(name);
});

test('the place record shows real dwell, and only where it exists', async ({
  page,
}) => {
  // THIS TEST REPLACES ONE ASSERTING THE OPPOSITE. It read "never reports
  // dwell it cannot measure" and checked the word "Dwell" was absent, because
  // both this pane and the taxonomy strip were built believing the product had
  // no per-place dwell. It has: `/v1/me/location/breakdown` returns
  // `dwell_seconds` per system / planet / city, and had done all along.
  //
  // The real rule is narrower and worth guarding: dwell appears where the
  // endpoint aggregates it, and NOT at site level, which is deeper than the
  // endpoint goes. There the pane shows the sighting span — the gap between a
  // stop's first and last sighting, which understates a visit and is zero for
  // a single sighting, so it must never wear the word "Dwell".
  await page.goto('/me/travel?level=city');
  await expect(page.locator('.hp-settings')).toBeVisible();
  await openGroup(page, 'Trail');
  await page.locator('.hp-rw a').first().click();

  const detail = page.locator('.hp-journeydetail');
  await expect(detail).toBeVisible();
  await expect(detail.locator('.hp-subs')).toContainText('Dwell');
  // Never a fabricated death count: the trace does not carry one.
  await expect(detail.locator('.hp-subs')).not.toContainText('Deaths');

  // A site is below the breakdown's deepest aggregate.
  await page.goto('/me/travel?level=site');
  await openGroup(page, 'Trail');
  await page.locator('.hp-rw a').first().click();
  const siteDetail = page.locator('.hp-journeydetail');
  await expect(siteDetail).toBeVisible();
  await expect(siteDetail.locator('.hp-subs')).toContainText('Sighting span');
  await expect(siteDetail.locator('.hp-subs')).not.toContainText('Dwell');
});

test('the taxonomy ranks by time spent, not by visit count', async ({
  page,
}) => {
  // `Journey.jsx` ranks places by hours. The port ranked by visits and said in
  // a comment that dwell was unavailable. The fixture is built so the two
  // orders DISAGREE — Lorville is visited once and dwelt in longest — because
  // a fixture where they agree cannot tell the two implementations apart.
  await page.goto('/me/travel?level=city');
  await expect(page.locator('.hp-settings')).toBeVisible();
  await openGroup(page, 'Trail');

  const plane = page.locator('.hp-plane', { hasText: 'Cities' }).first();
  await expect(plane).toContainText('by time spent');
  await expect(plane.locator('.hp-rw').first()).toContainText('Lorville');
});
