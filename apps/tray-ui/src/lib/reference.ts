/**
 * Tray-side fetcher for the StarStats reference catalogue.
 *
 * Mirrors the web app's `apps/web/src/lib/reference.ts` shape but
 * routes through a Tauri command (`get_reference_category`) rather
 * than a direct `fetch()`. The WebView's CSP restricts cross-origin
 * fetch from the frontend, so the catalogue listing has to come
 * Rust-side. The HTTP call itself uses the paired API URL
 * (`Config.remote_sync.api_url`).
 *
 * Returns empty Maps on any failure so callers — the Knowledge base
 * pane + the timeline prettifier — degrade to raw class names
 * rather than throwing.
 *
 * The server-side per-fetch rate limit is 10 req/s burst 40, so
 * cold-loading all four categories on app boot is fine.
 */

import { api } from '../api';

export type ReferenceCategory = 'vehicle' | 'weapon' | 'item' | 'location';

export const REFERENCE_CATEGORIES: ReadonlyArray<ReferenceCategory> = [
  'vehicle',
  'weapon',
  'item',
  'location',
];

/** Per-category curated summary, internally tagged by `category`.
 *  Mirrors the Rust `Summary` enum in
 *  `crates/starstats-server/src/reference_data.rs`. */
export type Summary =
  | { category: 'vehicle'; manufacturer?: string; role?: string; hull_size?: string; focus?: string }
  | { category: 'weapon'; manufacturer?: string; size?: string; damage_type?: string; weapon_type?: string }
  | { category: 'item'; manufacturer?: string; item_type?: string; grade?: string }
  | LocationSummary;

/** Location summary — Wave 1 (api.star-citizen.wiki) plus Wave 2
 *  (starcitizen.tools taxonomy enrichment). All Wave 2 fields are
 *  optional; populated when the server-side enrichment cron has
 *  matched the row. Mirrors `apps/web/src/lib/reference-types.ts`. */
export interface LocationSummary {
  category: 'location';
  // -- Wave 1 ----------------------------------------------------
  system?: string;
  parent?: string;
  tag?: string;
  classification?: string;
  // -- Wave 2 (taxonomy v2) -------------------------------------
  tier?: LocationTier;
  subtype?: LocationSubtype | string;
  placement?: Placement;
  operator?: string;
  faction?: string;
}

/** Coarse top-tier classification. Mirrors
 *  `starstats_core::location_taxonomy::LocationTier` and the web
 *  type union of the same name. */
export type LocationTier =
  | 'system'
  | 'astronomical_object'
  | 'landing_zone'
  | 'space_station'
  | 'landmark'
  | 'flotilla'
  | 'naval_base'
  | 'anonymous_poi';

/** Sub-bucket identifiers. Wire type stays `string` for forward-
 *  compat; this union enumerates the canonical set the tray
 *  renderer recognises. Keep in sync with the web mirror. */
export type LocationSubtype =
  | 'star'
  | 'planet'
  | 'moon'
  | 'planetoid'
  | 'nebula'
  | 'asteroid_belt'
  | 'city'
  | 'rest_stop'
  | 'orbital_station'
  | 'asteroid_base'
  | 'gateway_station'
  | 'orbital_laser_platform'
  | 'sealed_settlement'
  | 'outpost'
  | 'settlement'
  | 'spaceport'
  | 'salvage_yard'
  | 'drug_lab'
  | 'distribution_center'
  | 'racetrack'
  | 'convention_center'
  | 'shelter'
  | 'forward_operating_base'
  | 'hospital'
  | 'market'
  | 'bar'
  | 'restaurant'
  | 'museum'
  | 'apartment_hab'
  | 'lounge'
  | 'commercial_building'
  | 'planetary_alignment_facility'
  | 'geomorphology'
  | 'comm_array'
  | 'jump_point'
  | 'crash_site'
  | 'cave'
  | 'bunker'
  | 'derelict_ship';

/** Spatial relation to a parent body. Discriminated on `kind`. */
export type Placement =
  | { kind: 'on_body'; body: string }
  | { kind: 'orbits_body'; body: string }
  | { kind: 'lagrange_point'; lagrange: number; body: string }
  | { kind: 'sunward_from'; body: string }
  | { kind: 'angle_from'; degrees: number; body: string };

/** Human label for a tier. */
export function tierLabel(tier: LocationTier): string {
  switch (tier) {
    case 'system':
      return 'System';
    case 'astronomical_object':
      return 'Astronomical';
    case 'landing_zone':
      return 'Landing zone';
    case 'space_station':
      return 'Space station';
    case 'landmark':
      return 'Landmark';
    case 'flotilla':
      return 'Flotilla';
    case 'naval_base':
      return 'Naval base';
    case 'anonymous_poi':
      return 'Point of interest';
  }
}

/** Human label for a known subtype. Returns a reasonable
 *  title-cased fallback for unknown strings so a wiki-side
 *  addition still renders. */
export function subtypeLabel(subtype: LocationSubtype | string): string {
  const known: Partial<Record<LocationSubtype, string>> = {
    drug_lab: 'Drug lab',
    salvage_yard: 'Salvage yard',
    rest_stop: 'Rest stop',
    distribution_center: 'Distribution center',
    sealed_settlement: 'Sealed settlement',
    forward_operating_base: 'FOB',
    apartment_hab: 'Apartment hab',
    commercial_building: 'Commercial building',
    convention_center: 'Convention center',
    orbital_laser_platform: 'Orbital laser platform',
    orbital_station: 'Orbital station',
    asteroid_base: 'Asteroid base',
    gateway_station: 'Gateway station',
    asteroid_belt: 'Asteroid belt',
    planetary_alignment_facility: 'Planetary alignment',
    derelict_ship: 'Derelict ship',
    crash_site: 'Crash site',
    comm_array: 'Comm array',
    jump_point: 'Jump point',
  };
  return (
    known[subtype as LocationSubtype] ??
    subtype.replace(/_/g, ' ').replace(/\b\w/g, (c) => c.toUpperCase())
  );
}

/** Compact spatial-relation string. */
export function placementLabel(placement: Placement): string {
  switch (placement.kind) {
    case 'on_body':
      return `on ${placement.body}`;
    case 'orbits_body':
      return `orbits ${placement.body}`;
    case 'lagrange_point':
      return `L${placement.lagrange} of ${placement.body}`;
    case 'sunward_from':
      return `sunward from ${placement.body}`;
    case 'angle_from':
      return `${placement.degrees}° from ${placement.body}`;
  }
}

export interface ReferenceEntry {
  category: ReferenceCategory;
  class_name: string;
  display_name: string;
  slug: string | null;
  summary: Summary;
}

interface ListResponse {
  entries: Array<{
    class_name: string;
    display_name: string;
    slug?: string | null;
    summary?: Summary;
  }>;
}

function emptySummary(category: ReferenceCategory): Summary {
  switch (category) {
    case 'vehicle':
      return { category: 'vehicle' };
    case 'weapon':
      return { category: 'weapon' };
    case 'item':
      return { category: 'item' };
    case 'location':
      return { category: 'location' };
  }
}

export type ReferenceMap = ReadonlyMap<string, string>;
export type ReferenceCatalog = ReadonlyMap<string, ReferenceEntry>;

export interface CategoryBundle {
  map: ReferenceMap;
  catalog: ReferenceCatalog;
  list: ReferenceEntry[];
}

const EMPTY_BUNDLE: CategoryBundle = {
  map: new Map(),
  catalog: new Map(),
  list: [],
};

/** Strip a trailing `/` so we can safely concatenate route segments. */
function trimTrailingSlash(s: string): string {
  return s.replace(/\/+$/, '');
}

/**
 * Fetch a single category from the paired StarStats API via the
 * Rust client (so the WebView CSP doesn't block the request).
 *
 * Returns the empty bundle on any error so the UI keeps rendering;
 * the calling pane shows a friendly empty state in that case.
 */
export async function getCategoryBundle(
  apiUrl: string,
  category: ReferenceCategory,
): Promise<CategoryBundle> {
  if (!apiUrl) return EMPTY_BUNDLE;
  try {
    const body = (await api.getReferenceCategory(
      apiUrl,
      category,
    )) as ListResponse;
    const list: ReferenceEntry[] = [];
    const map = new Map<string, string>();
    const catalog = new Map<string, ReferenceEntry>();
    for (const e of body.entries ?? []) {
      if (!e.class_name || !e.display_name) continue;
      const entry: ReferenceEntry = {
        category,
        class_name: e.class_name,
        display_name: e.display_name,
        slug: e.slug ?? null,
        summary: e.summary ?? emptySummary(category),
      };
      list.push(entry);
      const classKey = e.class_name.toLowerCase();
      map.set(classKey, e.display_name);
      catalog.set(classKey, entry);
      // Also key the catalog by lower-cased display_name so callers
      // who hand in an already-friendly identifier (e.g.
      // `TraceEntry.city = "New Babbage"`) resolve to the same
      // entry. Skip when it collides with the class_name key so
      // the class_name entry remains canonical.
      const nameKey = e.display_name.toLowerCase();
      if (nameKey !== classKey && !catalog.has(nameKey)) {
        catalog.set(nameKey, entry);
      }
    }
    list.sort((a, b) => a.display_name.localeCompare(b.display_name));
    return { map, catalog, list };
  } catch {
    return EMPTY_BUNDLE;
  }
}

export interface AllReferenceBundles {
  vehicle: CategoryBundle;
  weapon: CategoryBundle;
  item: CategoryBundle;
  location: CategoryBundle;
}

export const EMPTY_ALL_BUNDLES: AllReferenceBundles = {
  vehicle: EMPTY_BUNDLE,
  weapon: EMPTY_BUNDLE,
  item: EMPTY_BUNDLE,
  location: EMPTY_BUNDLE,
};

/** Load all four categories in parallel. Each can degrade
 *  independently to an empty bundle. */
export async function loadAllReferenceBundles(
  apiUrl: string,
): Promise<AllReferenceBundles> {
  // `allSettled`, not `all`: one category failing (e.g. the large
  // ~2.5 MB item catalogue timing out / exceeding the IPC limit) must
  // NOT reject the whole load and leave every category empty (which
  // would strip links off ships/weapons/locations too). Each
  // `getCategoryBundle` already catches its own errors; this is the
  // load-bearing isolation so a single slow category can't blank the
  // rest.
  const settled = await Promise.allSettled(
    REFERENCE_CATEGORIES.map((c) => getCategoryBundle(apiUrl, c)),
  );
  const [vehicle, weapon, item, location] = settled.map((s) =>
    s.status === 'fulfilled' ? s.value : EMPTY_BUNDLE,
  );
  return { vehicle, weapon, item, location };
}

/** Build the web KB URL for a given entry. Falls back to a
 *  category-listing URL if the entry has no slug. Used by the tray
 *  pane to open the entity in the user's default browser via
 *  `@tauri-apps/plugin-shell`. */
export function webKbUrl(
  webOrigin: string,
  category: ReferenceCategory,
  entry: ReferenceEntry,
): string {
  const base = trimTrailingSlash(webOrigin);
  if (entry.slug) return `${base}/kb/${category}/${entry.slug}`;
  return `${base}/kb/${category}`;
}

/**
 * Variant / loaner suffixes appended to a base class name in some
 * event payloads but absent from the wiki catalogue. Mirrors the web
 * `apps/web/src/lib/reference-types.ts`. Stripped as a second lookup
 * attempt so `ARGO_MOLE_Teach` resolves to `ARGO_MOLE`. Lowercased.
 */
const VARIANT_SUFFIXES: readonly string[] = ['_teach', '_loaner'];

/**
 * Item class identifiers that are character-avatar parts, structural
 * placeholders, or engine defaults — never catalogued equipment. Keep
 * in sync with the web mirror. Match is case-insensitive.
 */
const NON_LINKABLE_ITEM_PATTERNS: readonly RegExp[] = [
  /^default(_|$)/i,
  /^head_/i,
  /^body_/i,
  /^shared_scalp/i,
  /^pu_protos/i,
  /^fp_visor$/i,
  /^fps_default/i,
  /lensdisplay/i,
];

/** True when an item class is avatar/structural noise. Pure. */
export function isNonLinkableItemClass(classKey: string): boolean {
  return NON_LINKABLE_ITEM_PATTERNS.some((re) => re.test(classKey));
}

const COSMETIC_ITEM_PORTS: readonly RegExp[] = [
  /^(eyes|hair|eyelashes|eyebrow|beard|teeth|head|face)_itemport$/i,
  /^body_itemport$/i,
  /_scalp/i,
];

/** True when an item PORT is avatar customisation / structural. Pure. */
export function isCosmeticItemPort(port: string | null | undefined): boolean {
  if (!port) return false;
  return COSMETIC_ITEM_PORTS.some((re) => re.test(port));
}

/**
 * Resolve a raw class identifier within a single category's catalog,
 * applying the item-noise filter and variant-suffix strip. Mirror of
 * the web `resolveReferenceEntry`. Pure; returns undefined on miss.
 */
export function resolveReferenceEntry(
  category: ReferenceCategory,
  classKey: string | null | undefined,
  catalog: ReferenceCatalog | undefined,
): ReferenceEntry | undefined {
  if (!classKey || !catalog) return undefined;
  if (category === 'item' && isNonLinkableItemClass(classKey)) {
    return undefined;
  }
  const key = classKey.toLowerCase();
  const direct = catalog.get(key);
  if (direct) return direct;
  if (category === 'location') return undefined;
  for (const suffix of VARIANT_SUFFIXES) {
    if (key.endsWith(suffix) && key.length > suffix.length) {
      const stripped = catalog.get(key.slice(0, -suffix.length));
      if (stripped) return stripped;
    }
  }
  return undefined;
}

/** Locate a class identifier across all four catalogues. Used by
 *  the ReactNode prettifier (`prettifySummaryReact`) — the regex
 *  picks tokens out of a server-rendered summary string without
 *  knowing which category they belong to, so we probe each catalog
 *  until we find the entry. Order matches `REFERENCE_CATEGORIES`
 *  (vehicle → weapon → item → location); ties shouldn't happen in
 *  practice because the wiki sync namespaces by category, but the
 *  iteration order is deterministic if they ever did.
 *
 *  Applies the item-noise filter + variant-suffix strip per category
 *  via `resolveReferenceEntry`, so loaner variants resolve and avatar
 *  noise doesn't bind.
 *
 *  Returns `null` when no catalogue claims the identifier — the
 *  caller falls back to the raw string in that case (same
 *  behaviour as the legacy `prettifySummary`). */
export function findEntityInBundles(
  classKey: string,
  bundles: AllReferenceBundles,
): { category: ReferenceCategory; entry: ReferenceEntry } | null {
  for (const category of REFERENCE_CATEGORIES) {
    const entry = resolveReferenceEntry(
      category,
      classKey,
      bundles[category].catalog,
    );
    if (entry) return { category, entry };
  }
  return null;
}
