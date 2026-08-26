/**
 * /kb — Knowledge base browse flow.
 *
 * M10 CUTOVER NOTE: the landing (`/kb`) and per-category (`/kb/[category]`)
 * pages now render from the BUILD-TIME static `reference-data` package
 * (`packages/reference-data/snapshots/*.json`), NOT a runtime
 * `/v1/reference/{category}` fetch — so the mock `kbListing` fixture no
 * longer feeds them. These tests therefore assert against the REAL
 * committed catalogue: the entry counts come from
 * `packages/reference-data/snapshots/manifest.json` and the example
 * ships are real Aegis / Origin / Drake vehicles from `vehicle.json`.
 *
 * The DETAIL page (`/kb/[category]/[slug]`) still fetches its full entry
 * via the per-slug runtime endpoint (`getEntityDetail`), so the
 * `kbDetail` mock fixture still applies there.
 *
 * Asserts:
 *   - Landing renders the four category tiles with their real static
 *     entry counts.
 *   - Per-category page renders real entries from the static catalogue,
 *     links each card to /kb/{category}/{slug}, and filters by the
 *     manufacturer facet chip.
 *   - Detail page renders the at-a-glance summary facts + the curated
 *     grouped metadata (specs) view.
 */

import { expect, test } from '@playwright/test';
import { kbDetail, resetScenario, setScenario } from './helpers/api-mock';

// Real committed counts — mirror packages/reference-data/snapshots/manifest.json
// (version 2026-07-21). If the snapshots are regenerated with a
// different vintage, update these to the new manifest counts.
const REAL_COUNTS = {
  vehicle: '295 entries',
  weapon: '409 entries',
  item: '12,282 entries', // toLocaleString inserts the thousands separator
  location: '1,954 entries',
} as const;

test.beforeEach(async ({ request }) => {
  await resetScenario(request);
});

test('kb_landing_renders_four_category_tiles_with_counts', async ({
  page,
}) => {
  // No scenario needed — the landing reads the static catalogue for its
  // counts. (Contracts + layout fetches degrade gracefully when unmocked.)
  await page.goto('/kb');

  await expect(
    page.getByRole('heading', { name: 'Knowledge base', level: 1 }),
  ).toBeVisible();

  // Each category shows its real static entry count. SCOPED to the catalogue
  // header, which is where the label and its count now sit together: a
  // page-wide `getByText('Vehicles', { exact: true })` matches three elements
  // since the header arrived (the count's label, the tab, the list row) and
  // resolves to a strict-mode violation rather than a useful assertion.
  const head = page.locator('.hp-cathead');
  for (const [label, count] of [
    ['Vehicles', REAL_COUNTS.vehicle],
    ['Weapons', REAL_COUNTS.weapon],
    ['Items', REAL_COUNTS.item],
    ['Locations', REAL_COUNTS.location],
  ] as const) {
    const stat = head.locator('.hp-subs > div', { hasText: label });
    await expect(stat, label).toHaveCount(1);
    // The header carries the bare figure; the word "entries" belongs to the
    // pane's own context line, not to every stat in the strip.
    await expect(stat, label).toContainText(count.replace(' entries', ''));
  }
});

test('kb_category_renders_entries_and_links_to_detail', async ({ page }) => {
  // Static catalogue — the /kb/vehicle list renders the real committed
  // vehicles, paginated (PAGE_SIZE=60) and sorted by display_name asc.
  await page.goto('/kb/vehicle');

  await expect(
    page.getByRole('heading', { name: 'Vehicles', level: 1 }),
  ).toBeVisible();

  // Each card is wrapped in a Link whose accessible name is the entry's
  // `display_name` (via aria-label) and whose href is
  // `/kb/vehicle/{slug}`. Assert two real page-1 entries (sorted asc:
  // "100i" is first; "Avenger Stalker" is well within the first 60).
  // Real display_name is "Avenger Stalker" — the manufacturer ("Aegis
  // Dynamics") is a separate summary field, NOT prefixed into the name.
  const oneHundredI = page.getByRole('link', { name: '100i', exact: true });
  await expect(oneHundredI).toBeVisible();
  await expect(oneHundredI).toHaveAttribute('href', '/kb/vehicle/100i');

  const avenger = page.getByRole('link', {
    name: 'Avenger Stalker',
    exact: true,
  });
  await expect(avenger).toBeVisible();
  await expect(avenger).toHaveAttribute('href', '/kb/vehicle/avenger-stalker');
});

test('kb_category_filters_by_manufacturer_facet_chip', async ({ page }) => {
  await page.goto('/kb/vehicle');

  // Manufacturer facet chips are built from the real catalogue (19
  // distinct manufacturers). Their accessible name is the manufacturer
  // value; the card preview text ("manufacturer: Aegis Dynamics") is not
  // a link, so a role=link exact-name query targets the chip only.
  const aegisChip = page.getByRole('link', {
    name: 'Aegis Dynamics',
    exact: true,
  });
  await expect(aegisChip).toBeVisible();
  await expect(
    page.getByRole('link', { name: 'Drake Interplanetary', exact: true }),
  ).toBeVisible();

  // Before filtering, a Drake ship ("Buccaneer", sorted idx 51 → page 1)
  // and an Aegis ship ("Avenger Stalker", idx 42) are both visible.
  const buccaneer = page.getByRole('link', { name: 'Buccaneer', exact: true });
  const avenger = page.getByRole('link', {
    name: 'Avenger Stalker',
    exact: true,
  });
  await expect(buccaneer).toBeVisible();
  await expect(avenger).toBeVisible();

  // Click the Aegis facet chip → the list narrows to Aegis Dynamics ships
  // only (34, all on the single page). The Aegis ship stays; the Drake
  // ship is filtered out entirely.
  await aegisChip.click();
  await expect(avenger).toBeVisible();
  await expect(buccaneer).toHaveCount(0);
});

test('kb_detail_renders_summary_and_full_metadata', async ({
  page,
  request,
}) => {
  // The detail page still fetches its entry via the runtime per-slug
  // endpoint, so the mock fixture drives it. `metadata` carries curated
  // numeric specs that map into the grouped Compact view (Flight &
  // handling / Survivability), replacing the removed "All raw fields"
  // dump.
  await setScenario(request, {
    __id: 'kb_detail',
    routes: {
      'GET /v1/reference/vehicle/slug/aegis-avenger-stalker': kbDetail({
        category: 'vehicle',
        class_name: 'AEGS_Avenger_Stalker',
        display_name: 'Aegis Avenger Stalker',
        slug: 'aegis-avenger-stalker',
        summary: {
          manufacturer: 'Aegis Dynamics',
          role: 'Fighter',
          hull_size: 'Small',
        },
        metadata: {
          manufacturer: { name: 'Aegis Dynamics', code: 'AEGS' },
          speed: { scm: 210 },
          health: 1000,
        },
      }),
    },
  });

  await page.goto('/kb/vehicle/aegis-avenger-stalker');

  await expect(
    page.getByRole('heading', { name: 'Aegis Avenger Stalker', level: 1 }),
  ).toBeVisible();
  // Raw class identifier chip in the hero.
  await expect(page.getByText('AEGS_Avenger_Stalker')).toBeVisible();

  // "At a glance" — the curated summary facts. `exact: true` on the
  // labels to avoid strict-mode collisions with metadata-derived rows.
  await expect(page.getByText('At a glance')).toBeVisible();
  await expect(page.getByText('Manufacturer', { exact: true })).toBeVisible();
  await expect(page.getByText('Aegis Dynamics').first()).toBeVisible();
  await expect(page.getByText('Hull size', { exact: true })).toBeVisible();

  // The former flat "All raw fields" dump was removed in M10; the
  // metadata now renders as curated, grouped spec sections in the
  // "Compact" view. Drive the view toggle like a user and assert a
  // metadata-derived spec renders.
  await page.getByRole('button', { name: 'Compact' }).click();
  await expect(
    page.getByRole('heading', { name: 'Flight & handling' }),
  ).toBeVisible();
  await expect(page.getByText('SCM speed')).toBeVisible();
  await expect(page.getByText('210 m/s')).toBeVisible();
});

test('kb_detail_renders_not_found_page_when_endpoint_returns_404', async ({
  page,
  request,
}) => {
  await setScenario(request, {
    __id: 'kb_detail_missing',
    routes: {
      'GET /v1/reference/vehicle/slug/no-such-ship': {
        status: 404,
        body: { error: 'entry_not_found' },
      },
    },
  });

  // Asserts user-visible behavior: when the slug endpoint 404s, the
  // detail page calls `notFound()` and Next renders the root
  // `not-found.tsx`. We assert the rendered content rather than the
  // HTTP status because Next 15's dev server doesn't propagate the
  // `notFound()` status correctly (it serves the not-found body with
  // HTTP 200 in dev; production builds correctly return 404). The
  // user-visible page is what matters for the e2e assertion.
  await page.goto('/kb/vehicle/no-such-ship');
  await expect(
    page.getByRole('heading', { name: 'Page not found', level: 1 }),
  ).toBeVisible();
});

test('kb_detail_rate_limited_renders_from_snapshot_instead_of_crashing', async ({
  page,
  request,
}) => {
  /**
   * A 429 IS NOT AN ERROR, and treating it as one crashed the page.
   *
   * The reference API is per-IP rate limited and the web container is one IP
   * fronting every SSR render, so a busy moment 429s legitimate navigations.
   * The detail page threw on any non-404 failure, so those readers got an
   * error boundary for entries that exist — beta's log was a wall of
   * `Failed to load item/…: 429 Too Many Requests`, one line per crash.
   *
   * The slug here is a REAL entry in the shipped `reference-data` snapshot,
   * because that is the point: the catalogue is compiled into the image, so a
   * rate-limited render still has the name, slug and classification in memory
   * and only the live-only blob (ship matrix, media) is missing.
   */
  await setScenario(request, {
    __id: 'kb_detail_429',
    routes: {
      'GET /v1/reference/vehicle/slug/avenger-stalker': {
        status: 429,
        body: { error: 'rate_limited' },
      },
    },
  });

  await page.goto('/kb/vehicle/avenger-stalker');

  // Not the error boundary. This is the assertion that fails on the old
  // behaviour — everything else about the page was already fine.
  await expect(page.locator('text=Something went wrong')).toHaveCount(0);
  // The entry still renders, from the snapshot.
  await expect(
    page.getByRole('heading', { name: /Avenger Stalker/i }).first(),
  ).toBeVisible();
  // And it says so, rather than quietly serving possibly-stale detail as
  // though it were live.
  await expect(page.locator('text=catalogue snapshot').first()).toBeVisible();
});

/**
 * The "Item type" row showed a machine identifier.
 *
 * Reported verbatim from the live site: "Odyssey II Undersuit Alpha" listed
 * its item type as `Char_Armor_Undersuit`, while the very same response
 * carried `metadata.classification_label = "Undersuit"`. The page was
 * rendering the raw `summary.item_type` with the human label sitting beside
 * it in the response it had already fetched.
 *
 * The unit tests cover the token prettifier, but a prettifier nothing calls
 * would pass those and change nothing on screen — this asserts the page.
 */
test('an item names its type in words, not an engine token', async ({
  page,
  request,
}) => {
  await resetScenario(request);
  await setScenario(request, {
    __id: 'kb_item_type',
    routes: {
      'GET /v1/reference/item/slug/odyssey-ii-undersuit-alpha': kbDetail({
        category: 'item',
        class_name: 'rsi_odyssey_undersuit_01_01_01',
        display_name: 'Odyssey II Undersuit Alpha',
        slug: 'odyssey-ii-undersuit-alpha',
        summary: {
          manufacturer: 'Roberts Space Industries',
          item_type: 'Char_Armor_Undersuit',
        },
        metadata: {
          classification: 'FPS.Armor.Undersuit',
          classification_label: 'Undersuit',
        },
      }),
    },
  });

  await page.goto('/kb/item/odyssey-ii-undersuit-alpha', {
    waitUntil: 'domcontentloaded',
    timeout: 40_000,
  });
  await expect(
    page.getByRole('heading', { name: 'Odyssey II Undersuit Alpha' }).first(),
  ).toBeVisible({ timeout: 20_000 });

  // The server already worked out the friendly name; use it.
  const body = page.locator('body');
  await expect(body).toContainText('Undersuit');
  await expect(body).not.toContainText('Char_Armor_Undersuit');
});
