/**
 * Server-side reference catalogue — built at BUILD TIME from the
 * static `reference-data` package (committed, minified JSON snapshots),
 * NOT from a runtime `/v1/reference/*` fetch.
 *
 * M10 FULL CUTOVER: the catalog builders (`getCategoryBundle`,
 * `loadAllReferenceBundles`, `getReferences`, `loadAllReferences`,
 * `getLocationCatalog`) consolidate the static snapshots into the exact
 * same `ReferenceCatalog` / `CategoryBundle` / `LocationCatalog` shapes
 * consumers already read — so `<EntityLink>`, `EntityHoverCard`, the KB
 * list/landing, and the location taxonomy map keep working unchanged,
 * but resolve names/slugs/summaries from build-time data. There is NO
 * fallback to the runtime API for the catalog: this deletes the
 * documented Next 2 MB data-cache warning + the Playwright cross-
 * scenario cache leak (both were listing-fetch symptoms) at the source.
 *
 * The snapshots redistribute FACTS ONLY (names / specs / taxonomy) —
 * the generator drops all prose — so the catalog carries no wiki-
 * copyrightable content (see `reference-data/src/attribution.ts`).
 *
 * Still runtime (per-slug, NOT the catalog listing): `getEntityDetail`
 * (KB detail metadata) and the peer-stats / compare endpoints, via
 * `apiBase()`.
 *
 * SERVER-ONLY: `apiBase()` reads `process.env.STARSTATS_API_URL` which
 * isn't exposed to the client bundle, and the parent `./api` is marked
 * `import 'server-only'`. Client components must import types /
 * constants / pure helpers from `./reference-types` instead — this
 * module re-exports everything from there so server callers don't need
 * a separate import path.
 */

import 'server-only';
import {
  loadConsolidatedCatalog,
  type ConsolidatedEntry,
} from 'reference-data';
import { apiBase } from './api';
import {
  CATEGORIES,
  emptySummary,
  type CategoryBundle,
  type EntityDetailOutcome,
  type LocationCatalog,
  type LocationEntry,
  type LocationSummary,
  type ReferenceCatalogs,
  type ReferenceCategory,
  type ReferenceEntry,
  type ReferenceEntryDetail,
  type ReferenceLookup,
  type ReferenceMap,
  type Summary,
} from './reference-types';

// Re-export the entire client-safe surface so server callers can
// keep importing everything from `@/lib/reference` without caring
// about the type/fetcher split. Client callers (use-client files)
// must import from `@/lib/reference-types` instead — pulling them
// through this module would taint the client bundle with
// `server-only`.
export * from './reference-types';

/**
 * Categories whose `/stats` response exceeds Next 15's hardcoded 2 MB
 * per-entry data-cache limit. Caching them logs
 *   "Failed to set Next.js data cache for … items over 2MB can not
 *    be cached (N bytes)"
 * on every render and pays the serialization cost for nothing.
 *
 * Measured 2026-08-12: `vehicle/stats` = 2,250,307 bytes.
 *
 * HISTORY, because the previous shape of this caused an outage-ish
 * bug. This was once `LARGE_CATEGORIES`, keyed on the category's full
 * LISTING (`/v1/reference/{category}`, ~3.4 MB for item, ~4 MB for
 * vehicle). That listing is no longer fetched at runtime — it ships in
 * a static package — so the set's only surviving effect was disabling
 * the data cache on the small per-slug DETAIL reads for item and
 * vehicle. Every KB detail render then hit the API uncached, and a
 * Next prefetch burst turned into a wave of uncached calls that tripped
 * the per-IP reference governor and 429'd. The escape hatch outlived
 * the problem it solved and inverted into its own failure mode.
 *
 * So the decision is now keyed on the ENDPOINT, not the category. If a
 * new category's stats crosses 2 MB, add it here; the symptom is the
 * cache-warning log line above. Do NOT re-key this on the category.
 */
/** @internal exported for testing */
export const LARGE_STATS_CATEGORIES: ReadonlySet<ReferenceCategory> = new Set([
  'vehicle',
]);

/**
 * Which reference endpoint a fetch is for.
 *
 * `detail` — `/v1/reference/{category}/slug/{slug}`, one entry. Always
 *            small, always worth caching.
 * `stats`  — `/v1/reference/{category}/stats`, quantile summaries.
 *            Small for most categories; see `LARGE_STATS_CATEGORIES`.
 */
export type KbEndpoint = 'detail' | 'stats';

/**
 * Per-fetch cache directive for reference endpoints.
 *
 * Production: 1h revalidate — wiki sync is daily, an hour stale is
 * invisible. The only exception is an oversized `/stats` payload, which
 * cannot be cached at all.
 *
 * Playwright e2e: cache leaks between scenarios because Next holds
 * `revalidate` responses across `page.goto()` calls within one
 * dev-server lifetime. `STARSTATS_DISABLE_FETCH_CACHE=1` (set by the
 * Playwright webServer env) flips everything to `no-store` so each
 * scenario's `setScenario` is honoured by the next render.
 */
/** @internal exported for testing */
export function kbCacheOpts(
  endpoint: KbEndpoint,
  category?: ReferenceCategory,
): RequestInit {
  if (process.env.STARSTATS_DISABLE_FETCH_CACHE === '1') {
    return { cache: 'no-store' };
  }
  if (endpoint === 'stats' && category && LARGE_STATS_CATEGORIES.has(category)) {
    return { cache: 'no-store' };
  }
  return { next: { revalidate: 3600 } } as RequestInit;
}

/**
 * Fetch one category's entries and reduce to a `Map<lowercased
 * class_name, display_name>`. Returns an empty Map on any error so
 * callers get a stable shape rather than a thrown exception —
 * reference data is opt-in cosmetic, not load-bearing.
 *
 * Callers that also need slug + summary should use
 * [`getCategoryBundle`] instead — it shares the same network call.
 */
export async function getReferences(
  category: ReferenceCategory,
): Promise<ReferenceMap> {
  const bundle = await getCategoryBundle(category);
  return bundle.map;
}

/** Project one static consolidated entry into the runtime
 *  `ReferenceEntry` shape consumers read. The package's `summary` is an
 *  open facts record; it IS the app's `Summary` union at runtime (the
 *  generator writes the same per-category factual fields), so we cast
 *  rather than re-map, falling back to the empty per-category summary
 *  only if a snapshot row somehow lacks one. */
function toReferenceEntry(
  category: ReferenceCategory,
  e: ConsolidatedEntry,
): ReferenceEntry {
  return {
    category,
    class_name: e.className,
    display_name: e.displayName,
    slug: e.slug ?? null,
    summary: (e.summary as Summary | undefined) ?? emptySummary(category),
  };
}

/**
 * Build one category's `CategoryBundle` from the static snapshots.
 *
 * DUAL-KEYING PRESERVED (per docs/ENGINEERING.md): the `catalog` map stores each
 * entry under BOTH `class_name.toLowerCase()` AND
 * `display_name.toLowerCase()` (when the two keys differ and the name
 * key is free), so `<EntityLink>` resolves whether the caller hands in
 * a raw identifier (`CRU_LEO`) or an already-friendly one
 * (`New Babbage`). `list` holds exactly one entry per class_name (the
 * deduplicated view) and `map` is class_name → display_name. This is
 * byte-for-byte the same construction the old runtime fetcher used —
 * only the row source changed (static package vs `/v1/reference/*`).
 *
 * `async` is retained for call-site compatibility; the work is
 * synchronous (the package memoises its parsed snapshots).
 */
export async function getCategoryBundle(
  category: ReferenceCategory,
): Promise<CategoryBundle> {
  const rows = loadConsolidatedCatalog().byCategory[category] ?? [];
  const map = new Map<string, string>();
  const catalog = new Map<string, ReferenceEntry>();
  const list: ReferenceEntry[] = [];
  for (const row of rows) {
    if (!row.className || !row.displayName) continue;
    const classKey = row.className.toLowerCase();
    const entry = toReferenceEntry(category, row);
    map.set(classKey, entry.display_name);
    catalog.set(classKey, entry);
    list.push(entry);
    // Also key by lower-cased display_name so callers that already
    // hold a friendly identifier resolve to the same entry. `list`
    // only receives the class_name path so iteration yields each entry
    // once. Skip when the name key collides with the class_name key so
    // the class_name entry stays canonical.
    const nameKey = entry.display_name.toLowerCase();
    if (nameKey !== classKey && !catalog.has(nameKey)) {
      catalog.set(nameKey, entry);
    }
  }
  return { map, catalog, list };
}

/**
 * Fetch a single entry by slug. Returns a discriminated outcome so
 * the detail page can distinguish "no such entity" (→ `notFound()`)
 * from "the backend hiccupped" (→ error boundary or retry surface)
 * — collapsing both to null causes transient outages to render a
 * misleading permanent 404.
 */
export async function getEntityDetail(
  category: ReferenceCategory,
  slug: string,
): Promise<EntityDetailOutcome> {
  // Retry a 429 a couple of times with a short backoff. The public
  // reference API is per-IP rate-limited, and the web container is a
  // single IP fronting every SSR request — so a transient burst (a few
  // concurrent renders) can briefly 429 a legitimate navigation.
  // Without this, that transient limit becomes a hard page crash via
  // the detail page's throw-on-error. 404/other errors don't retry.
  const RETRYABLE_DELAYS_MS = [150, 400];
  try {
    for (let attempt = 0; ; attempt++) {
      const resp = await fetch(
        `${apiBase()}/v1/reference/${category}/slug/${encodeURIComponent(slug)}`,
        {
          method: 'GET',
          ...kbCacheOpts('detail', category),
        },
      );
      if (resp.status === 404) return { kind: 'not_found' };
      if (resp.status === 429 && attempt < RETRYABLE_DELAYS_MS.length) {
        await new Promise((r) => setTimeout(r, RETRYABLE_DELAYS_MS[attempt]));
        continue;
      }
      if (!resp.ok) {
        const reason = `${resp.status} ${resp.statusText}`;
        console.error(`reference ${category}/slug/${slug} returned ${reason}`);
        return { kind: 'error', reason };
      }
      const body = (await resp.json()) as ReferenceEntryDetail | null;
      if (!body) return { kind: 'not_found' };
      return { kind: 'ok', entry: body };
    }
  } catch (err) {
    console.error(`reference ${category}/slug/${slug} fetch failed`, err);
    return { kind: 'error', reason: String(err) };
  }
}

/**
 * Load all four reference categories in parallel. Each individual
 * fetch can degrade independently — a Weapon-only outage doesn't
 * affect Vehicle / Item / Location lookups.
 *
 * Returns the legacy display-name `ReferenceLookup` only; new
 * callers that need slug + summary should use
 * [`loadAllReferenceBundles`] instead.
 */
export async function loadAllReferences(): Promise<ReferenceLookup> {
  const { lookup } = await loadAllReferenceBundles();
  return lookup;
}

export interface AllReferenceBundles {
  lookup: ReferenceLookup;
  catalogs: ReferenceCatalogs;
  counts: Record<ReferenceCategory, number>;
}

// ---------------------------------------------------------------------
// Process-level bundle memo.
//
// The bundles are now built from the static `reference-data` snapshots
// (no network, no rate-limit, no Next data cache), so the elaborate
// TTL + in-flight-dedup + failure-backoff machinery the runtime fetcher
// needed is gone. We keep a trivial once-built memo so repeated renders
// reuse the same Map objects (the underlying package already memoises
// its parsed snapshots; this just avoids rebuilding the dual-keyed Maps
// per call).
let bundleCache: AllReferenceBundles | null = null;

/**
 * Reset the in-process reference-bundle memo. Test-only hook — vitest
 * needs a clean slate between cases since the memo is module-level
 * state that otherwise leaks across tests.
 * @internal
 */
export function __resetReferenceBundleCacheForTests(): void {
  bundleCache = null;
}

/** Load both `ReferenceLookup` (display-name maps) and
 *  `ReferenceCatalogs` (rich slug/summary catalogs) for all four
 *  categories, built from the static snapshots. Also returns
 *  per-category entry counts — derived from the deduplicated `list`
 *  because the catalog Map stores each entry under both class_name and
 *  display_name keys, so `catalog.size` would double-count.
 *
 *  Memoised in-process (see note above). */
export async function loadAllReferenceBundles(): Promise<AllReferenceBundles> {
  if (bundleCache) return bundleCache;
  const [vehicles, weapons, items, locations] = await Promise.all(
    CATEGORIES.map((c) => getCategoryBundle(c)),
  );
  bundleCache = {
    lookup: {
      vehicles: vehicles.map,
      weapons: weapons.map,
      items: items.map,
      locations: locations.map,
    },
    catalogs: {
      vehicles: vehicles.catalog,
      weapons: weapons.catalog,
      items: items.catalog,
      locations: locations.catalog,
    },
    counts: {
      vehicle: vehicles.list.length,
      weapon: weapons.list.length,
      item: items.list.length,
      location: locations.list.length,
    },
  };
  return bundleCache;
}

// -- Location catalog (catalog-driven hierarchy) ----------------------
//
// Locations get a richer treatment than the other categories: the
// catalogue carries the full system → body → place hierarchy as facts
// (system / parent / tag / classification), which `parseLocationClass`
// consults before falling back to the hardcoded engine-short-code
// dictionaries (see docs/REFERENCE-CATALOG-HIERARCHY.md).

/**
 * Build the location catalogue with hierarchy facts from the STATIC
 * snapshots — same multi-index shape (`byName` / `byTag` / `bySlug` /
 * `display` / `slugByClass`) the runtime fetcher produced, so
 * `parseLocationClass` and the travel widget keep working unchanged.
 * The indexed fields (system / parent / tag / classification / slug)
 * are exactly the `LocationSummary` facts the generator carries.
 *
 * `async` retained for call-site compatibility; the work is synchronous.
 */
export async function getLocationCatalog(): Promise<LocationCatalog> {
  const rows = loadConsolidatedCatalog().byCategory.location ?? [];
  const byName = new Map<string, LocationEntry>();
  const byTag = new Map<string, LocationEntry>();
  const bySlug = new Map<string, LocationEntry>();
  const display = new Map<string, string>();
  const slugByClass = new Map<string, string>();
  for (const row of rows) {
    if (!row.className) continue;
    // The package summary is the open facts record; narrow to the
    // location shape defensively (always `category: 'location'` here).
    const s = (row.summary as LocationSummary | undefined) ?? {
      category: 'location',
    };
    const entry: LocationEntry = {
      classKey: row.className,
      displayName: row.displayName || row.className,
      system: s.system?.trim() || null,
      parent: s.parent?.trim() || null,
      tag: s.tag?.trim() || null,
      slug: row.slug?.trim() || null,
      classification: s.classification?.trim() || null,
    };
    display.set(row.className.toLowerCase(), entry.displayName);
    if (row.displayName) {
      byName.set(row.displayName.toLowerCase(), entry);
    }
    if (entry.tag) byTag.set(entry.tag.toLowerCase(), entry);
    if (entry.slug) {
      bySlug.set(entry.slug.toLowerCase(), entry);
      slugByClass.set(row.className.toLowerCase(), entry.slug);
    }
  }
  return { byName, byTag, bySlug, display, slugByClass };
}
