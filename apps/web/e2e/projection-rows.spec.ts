import { test, expect, type Page } from '@playwright/test';
import { loginAs, resetScenario, scenarioFor, setScenario } from './helpers/api-mock';

/**
 * A ranked row is either a target or it is not, and it must look like whichever
 * it is.
 *
 * The fault this guards against was reported as "the items in lists are not
 * clickable and the clickable parts are not working properly", and measuring a
 * rendered `/me` showed exactly why. Every `.hp-rw` carried `cursor: pointer`
 * and a hover highlight, while the anchor inside it wrapped only the LABEL:
 *
 *     "Avenger Stalker"  anchor 91px of a 529px row — 10% of the area
 *     "300i"             anchor 26px of a 529px row —  3%
 *     "Totally Unknown"  no anchor at all —             0%, still pointer
 *
 * So a reader aiming at an obviously-clickable row missed nine times in ten,
 * and one row in the list was never clickable at all while claiming to be.
 *
 * Stretching the anchor from inside cannot fix it — `.nm` is `overflow: hidden`
 * so an `inset: 0` overlay is clipped back to the label — so `MeterRow` renders
 * the ROW as the anchor when it has an href.
 *
 * WHY THESE ASSERTIONS AND NOT THE OBVIOUS ONE. "The link is visible" and "the
 * link has the right href" both passed on the broken build: the anchor was
 * present, visible and correctly addressed. It was 3% of the row. The only
 * thing that separates fixed from broken is where a click LANDS, so the test
 * clicks the far edge — the part of the row the old anchor never covered — and
 * reads the cursor on a row that leads nowhere.
 */
const SCENARIO: Record<string, unknown> = {
  // `locations` is `enabled: false` in DEFAULT_LAYOUT, so "Places visited"
  // does not render without an explicit layout — the reader in the report has
  // it switched on. The other three are the planes the rest of this file
  // measures, and naming all four keeps them in one declared order.
  'GET /v1/users/me/profile-layout': {
    status: 200,
    body: {
      layout: [
        { id: 'fleet', enabled: true, size: 'compact' },
        { id: 'docking', enabled: true, size: 'compact' },
        { id: 'routes', enabled: true, size: 'compact' },
        { id: 'locations', enabled: true, size: 'compact' },
      ],
    },
  },
  'GET /v1/me/stats/fleet': {
    status: 200,
    body: {
      ships: [
        { vehicle_class: 'AEGS_Avenger_Stalker', trip_count: 12 },
        { vehicle_class: 'ORIG_300i', trip_count: 9 },
        // Deliberately not in the catalogue: this is the row that must NOT
        // advertise itself as a target.
        { vehicle_class: 'TOTALLY_UNKNOWN_HULL', trip_count: 4 },
      ],
    },
  },
  // All three rows here are berth KINDS, not entities — so this plane's rows
  // are every one of them unlinked, which is the case the rank sweep needs.
  'GET /v1/me/stats/docking': {
    status: 200,
    // `total_stows`, not `total` — the widget bails when it reads 0, so a
    // fixture with the wrong field name renders no plane and the sweep below
    // silently loses the only case it exists to cover.
    body: {
      total_stows: 30,
      by_kind: { hangar: 18, pad: 9, other: 3 },
      by_size: {},
    },
  },
  'GET /v1/me/stats/routes': {
    status: 200,
    body: {
      routes: [
        { destination: 'ArcCorp', count: 14 },
        { destination: 'Crusader', count: 9 },
      ],
    },
  },
  // THE RAW SHAPE THE API ACTUALLY RETURNS: `system|planet|city`, with empty
  // segments where the reader never got that specific. Taken verbatim from a
  // real account's Travel lens, where these rendered on screen as
  // `Stanton|clio|`, `Stanton|microTech|New Babbage` and a bare `||`.
  'GET /v1/me/stats/locations': {
    status: 200,
    body: {
      hours: 8760,
      unique_locations: 5,
      top_locations: [
        { value: 'Stanton|clio|', count: 128 },
        { value: 'Stanton|microTech|New Babbage', count: 27 },
        { value: 'Rr||mic Leo', count: 10 },
        { value: '||', count: 3 },
      ],
    },
  },
};

/**
 * Open `/me` under the Travel lens with its ranked planes drawn.
 *
 * The lens control only exists once the projection has mounted, and the planes
 * arrive with the server render, so both waits are on the elements themselves
 * rather than on a timeout — a fixed sleep passes vacuously on a page that
 * rendered nothing, which is exactly how the first draft of this file reported
 * three green rows it had never seen.
 */
async function openTravelLens(page: Page) {
  await page.goto('/me', { waitUntil: 'domcontentloaded', timeout: 30_000 });
  const travel = page.locator('.hp-lens button', { hasText: 'Travel' });
  await expect(travel).toBeVisible({ timeout: 20_000 });
  // RETRIED, because the lens control is server-rendered and the click only
  // does anything once React has attached to it. A single click lands on the
  // markup before hydration often enough that this file failed on a different
  // test each run — the default lens draws no ranked planes, so the symptom
  // was "no rows at all" rather than anything to do with rows.
  await expect(async () => {
    await travel.click();
    await expect(page.locator('.hp-rw').first()).toBeVisible({ timeout: 3_000 });
  }).toPass({ timeout: 40_000 });
}

test.describe('projection rows', () => {
  test.beforeEach(async ({ request }) => {
    await resetScenario(request);
    await setScenario(request, scenarioFor('projection-rows', SCENARIO));
  });

  test('a click at the far edge of a linked row navigates', async ({ page }) => {
    test.slow();
    await loginAs(page, { handle: 'TestPilot' });
    await openTravelLens(page);

    const row = page
      .locator('.hp-rw')
      .filter({ hasText: 'Avenger Stalker' })
      .first();
    await expect(row).toBeVisible();

    // The row itself carries the href — not a descendant. This is the
    // structural half of the fix, and it is what makes the whole row a target.
    await expect(row).toHaveAttribute('href', '/kb/vehicle/avenger-stalker');

    const box = await row.boundingBox();
    expect(box).not.toBeNull();
    // 30px in from the RIGHT edge: inside the value column, far outside where
    // the old label-only anchor reached.
    await page.mouse.click(box!.x + box!.width - 30, box!.y + box!.height / 2);
    // A generous budget on purpose: in `next dev` this is the first visit to
    // `/kb/[category]/[slug]`, so the click waits on a cold route compile. The
    // navigation is not slow in production and nothing here measures speed.
    await page.waitForURL('**/kb/vehicle/avenger-stalker', { timeout: 30_000 });
  });

  test('a row that leads nowhere does not offer a pointer', async ({ page }) => {
    await loginAs(page, { handle: 'TestPilot' });
    await openTravelLens(page);

    const dead = page
      .locator('.hp-rw')
      .filter({ hasText: 'Totally Unknown Hull' })
      .first();
    await expect(dead).toBeVisible();
    await expect(dead).not.toHaveAttribute('href', /./);
    // The affordance is the assertion. A row with nowhere to go that shows a
    // pointer and lifts on hover is a promise the page cannot keep.
    await expect(dead).toHaveCSS('cursor', 'auto');
  });

  test('every plane lights exactly its own leading rank', async ({ page }) => {
    await loginAs(page, { handle: 'TestPilot' });
    await openTravelLens(page);

    // SWEEPS EVERY PLANE, and the reason is worth keeping. `:first-of-type`
    // matches the first element of each TAG among its siblings, and a row is
    // an <a> when it links and a <div> when it does not. A plane's `.cap` is
    // also a <div> — so in a plane whose rows are ALL unlinked, the cap takes
    // the first-div slot and NO row is lit at all, while a plane starting with
    // a link looks perfectly correct. Checking one plane proves nothing; the
    // first version of this test passed against the broken selector for
    // exactly that reason.
    const planes = await page.evaluate(() =>
      [...document.querySelectorAll('.hp-plane')]
        .map((p) => ({
          cap: p.querySelector('.cap')?.textContent?.trim().slice(0, 24) ?? '?',
          ranks: [...p.querySelectorAll('.hp-rw')].map((r) => {
            const rk = r.querySelector('.rk');
            return rk ? getComputedStyle(rk).color : '';
          }),
        }))
        .filter((p) => p.ranks.length > 1),
    );
    expect(planes.length).toBeGreaterThan(1);
    for (const p of planes) {
      const [first, ...rest] = p.ranks;
      expect(rest, `${p.cap}: no row after the first may be lit`).not.toContain(
        first,
      );
    }
  });

  test('a place row reads as a place, and goes to it', async ({ page }) => {
    /**
     * `top_locations[].value` is a `system|planet|city` key, not a name. The
     * plane rendered it raw, so the Travel lens listed `Stanton|clio|`,
     * `Stanton|microTech|New Babbage` and a bare `||` — and since a composite
     * key matches nothing in a catalogue keyed by class and display name, not
     * one of those rows could link either. Reported as "the items in the
     * lists aren't clickable"; the labels were the other half of it.
     *
     * The resolution is `aggregateLocationBuckets`, which the flat widgets
     * already ran. This asserts the projection runs it too.
     */
    await loginAs(page, { handle: 'TestPilot' });
    await openTravelLens(page);

    const places = page.locator('.hp-plane', { hasText: 'Places visited' });
    await expect(places).toBeVisible();

    // No pipe survives to the screen.
    await expect(places).not.toContainText('|');
    // The composite keys became the places they name.
    await expect(places).toContainText('Clio');
    await expect(places).toContainText('New Babbage');
    // An empty key is named, not blank.
    await expect(places).toContainText('Unknown');

    // And a real place is now a destination, because the resolved label is
    // what the catalogue is keyed by.
    const clio = places.locator('.hp-rw').filter({ hasText: 'Clio' }).first();
    await expect(clio).toHaveAttribute('href', '/kb/location/clio');
  });

  test('every row with a link is itself the link', async ({ page }) => {
    await loginAs(page, { handle: 'TestPilot' });
    await openTravelLens(page);
    // Without this the sweep below passes on a page with no rows at all — the
    // exact vacuous green this file is supposed to prevent.
    expect(await page.locator('.hp-rw').count()).toBeGreaterThan(3);

    // Sweeps every rendered row rather than the two the tests above name, so a
    // future plane that reintroduces a nested anchor is caught by this file
    // rather than by a reader.
    const nested = await page.evaluate(() =>
      [...document.querySelectorAll('.hp-rw')]
        .filter((r) => r.querySelector('a[href]'))
        .map((r) => r.textContent?.trim().slice(0, 40) ?? ''),
    );
    expect(nested, 'a row must BE the anchor, not contain one').toEqual([]);
  });
});

/**
 * The loadout plane, which rendered an EMPTY value column on every row.
 *
 * `elements.tsx` declared the preview item as `{ label?, name?, count? }`; the
 * widget returns `{ class, label, category, slug }`. So `count` was always
 * undefined — hence the blank column — and the `slug` the widget had ALREADY
 * resolved went unused, which is why loadout rows led nowhere. The `as (d:
 * never)` cast in `BUILDERS` is what let a local interface disagree with its
 * source and still compile.
 *
 * Asserted as "the column has text in it", because that is the whole
 * difference. Every structural gate passed on the blank version: the rows were
 * present, the right shape, correctly classed and correctly counted.
 */
const LOADOUT_SCENARIO: Record<string, unknown> = {
  // The loadout widget is `enabled: false` in DEFAULT_LAYOUT, so it does not
  // render on /me without an explicit layout.
  'GET /v1/users/me/profile-layout': {
    status: 200,
    body: { layout: [{ id: 'loadout', enabled: true, size: 'compact' }] },
  },
  'GET /v1/me/events': {
    status: 200,
    body: {
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
              { class: 'behr_rifle_ballistic_01', port: 'weapon_body_primary', category: 'Weapon_FPS_Rifle' },
              { class: 'medpen_01', port: 'utility_attach_01', category: 'Char_Consumable_Medical' },
              { class: 'frag_grenade_01', port: 'grenade_attach_left', category: 'Char_Consumable_Grenade' },
            ],
          },
        },
      ],
    },
  },
  'POST /v1/reference/resolve': {
    status: 200,
    body: {
      resolved: {
        behr_rifle_ballistic_01: {
          display_name: 'BEHR P8-AR Rifle',
          slug: 'behr-p8-ar',
          category: 'Weapon_FPS_Rifle',
          classification: 'FPS.Weapon.Rifle',
          classification_label: 'Rifle',
          has_image: false,
        },
        medpen_01: {
          display_name: 'MedPen',
          slug: 'medpen',
          category: 'Char_Consumable_Medical',
          classification: 'FPS.Consumable.Medical',
          classification_label: 'Medical',
          has_image: false,
        },
        frag_grenade_01: {
          display_name: 'FS-9 Frag Grenade',
          slug: 'fs-9-frag',
          category: 'Char_Consumable_Grenade',
          classification: null,
          classification_label: null,
          has_image: false,
        },
      },
    },
  },
  'GET /v1/me/stats/loadout-activity': {
    status: 200,
    body: { equips: 73, stores: 28, top_items: [] },
  },
};

test.describe('loadout plane', () => {
  test.beforeEach(async ({ request }) => {
    await resetScenario(request);
    await setScenario(request, scenarioFor('projection-loadout', LOADOUT_SCENARIO));
  });

  test('every row names the kit and says what it is', async ({ page }) => {
    test.slow();
    await loginAs(page, { handle: 'TestPilot' });
    await page.goto('/me', { waitUntil: 'domcontentloaded', timeout: 30_000 });
    const lens = page.locator('.hp-lens button', { hasText: 'Loadout' });
    await expect(lens).toBeVisible({ timeout: 20_000 });
    await expect(async () => {
      await lens.click();
      await expect(page.locator('.hp-rw').first()).toBeVisible({ timeout: 3_000 });
    }).toPass({ timeout: 40_000 });

    const rows = page.locator('.hp-rw');
    expect(await rows.count()).toBeGreaterThan(1);
    // The name resolved, rather than showing the engine class id.
    await expect(rows.first()).toContainText('BEHR P8-AR Rifle');
    // And the value column is not blank. This is the assertion that fails on
    // the old shape; everything else about the row was already correct.
    const values = await page.locator('.hp-rw .vv').allTextContents();
    expect(values.length).toBeGreaterThan(1);
    for (const v of values) expect(v.trim()).not.toBe('');

    // The widget resolved a slug for each of these, so each row leads to it.
    await expect(rows.first()).toHaveAttribute('href', /^\/kb\/(weapon|item)\//);
  });
});
