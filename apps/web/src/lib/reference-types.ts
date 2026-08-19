/**
 * Client-safe types + pure helpers for the reference catalogue.
 *
 * Split out of `./reference` so client components (use-client files
 * like `EntityLink`, `EntityHoverCard`, `event-summary-react`) can
 * import the type surface without dragging in the server-only
 * fetchers — `./reference` imports `apiBase` from `./api`, which is
 * marked `import 'server-only'` for token-handling safety. Without
 * the split, importing any type from `./reference` taints the
 * client bundle and `next build` refuses to compile.
 *
 * The server-side wrapper `./reference` re-exports every symbol
 * from this module, so server callers keep working unchanged.
 */

import { toFriendlyName } from './heuristic-name';

export type ReferenceCategory = 'vehicle' | 'weapon' | 'item' | 'location';

export const CATEGORIES: ReadonlyArray<ReferenceCategory> = [
  'vehicle',
  'weapon',
  'item',
  'location',
];

/** Per-category curated summary, internally tagged by `category`
 *  for discriminated narrowing. Mirrors the Rust `Summary` enum in
 *  `crates/starstats-server/src/reference_data.rs`. Each variant's
 *  fields are optional — empty / missing fields are stripped on
 *  the wire (`skip_serializing_if = "Option::is_none"`). */
export type Summary =
  | VehicleSummary
  | WeaponSummary
  | ItemSummary
  | LocationSummary;

export interface VehicleSummary {
  category: 'vehicle';
  manufacturer?: string;
  role?: string;
  hull_size?: string;
  focus?: string;
}

export interface WeaponSummary {
  category: 'weapon';
  manufacturer?: string;
  size?: string;
  damage_type?: string;
  weapon_type?: string;
}

export interface ItemSummary {
  category: 'item';
  manufacturer?: string;
  item_type?: string;
  grade?: string;
}

export interface LocationSummary {
  category: 'location';
  // -- Wave 1 (api.star-citizen.wiki) ----------------------------
  system?: string;
  parent?: string;
  tag?: string;
  classification?: string;
  // -- Wave 2 (starcitizen.tools enrichment, Phase 1 rollout) ---
  // See docs/PLAN-LOCATION-TAXONOMY-V2.md. All optional;
  // populated when the server-side enrichment cron finds a
  // matching wiki page. Snake-case mirrors the Rust
  // `LocationTier` enum + open-ended subtype allow-list.
  tier?: LocationTier;
  subtype?: LocationSubtype | string;
  placement?: Placement;
  operator?: string;
  faction?: string;
}

/** Coarse top-tier classification. Mirrors
 *  `starstats_core::location_taxonomy::LocationTier`. Adding a
 *  variant requires updating the Rust enum + the
 *  `reference_registry_tier_chk` constraint in migration 0039 in
 *  lockstep. */
export type LocationTier =
  | 'system'
  | 'astronomical_object'
  | 'landing_zone'
  | 'space_station'
  | 'landmark'
  | 'flotilla'
  | 'naval_base'
  | 'anonymous_poi';

/** Known sub-bucket identifiers. The wire type is `string` for
 *  forward-compat (the wiki adds sub-buckets independently); this
 *  union enumerates the canonical set the renderer recognises. */
export type LocationSubtype =
  // Astronomical
  | 'star'
  | 'planet'
  | 'moon'
  | 'planetoid'
  | 'nebula'
  | 'asteroid_belt'
  // Landing zone
  | 'city'
  // Space station
  | 'rest_stop'
  | 'orbital_station'
  | 'asteroid_base'
  | 'gateway_station'
  | 'orbital_laser_platform'
  | 'sealed_settlement'
  // Landmark
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
  // Anonymous POI (engine-only, no wiki entry)
  | 'comm_array'
  | 'jump_point'
  | 'crash_site'
  | 'cave'
  | 'bunker'
  | 'derelict_ship';

/** Spatial relation to a parent body. Discriminated on `kind` —
 *  TS narrows automatically with `placement.kind === 'on_body'`.
 *  Mirrors the Rust `Placement` enum (and the server's
 *  `PlacementSchema` mirror). */
export type Placement =
  | { kind: 'on_body'; body: string }
  | { kind: 'orbits_body'; body: string }
  | { kind: 'lagrange_point'; lagrange: number; body: string }
  | { kind: 'sunward_from'; body: string }
  | { kind: 'angle_from'; degrees: number; body: string };

/** Human label for a tier — for chip rendering + filter UI. */
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

/** Human label for a known subtype. Returns the raw value for
 *  unknown strings so a wiki-side addition still renders something
 *  sensible before the renderer is updated. */
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

/** Compact spatial-relation string for hover-cards and detail
 *  tooltips. */
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

/** Empty/default summary for a given category — used when the
 *  server returns nothing (or hasn't synced yet). Lets consumers
 *  rely on `summary.category` being present even on a freshly-
 *  defaulted entry. */
export function emptySummary(category: ReferenceCategory): Summary {
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

/** Slim per-entry shape returned by `/v1/reference/{category}`. */
export interface ReferenceEntry {
  category: ReferenceCategory;
  class_name: string;
  display_name: string;
  /** URL-safe canonical identifier. Null on legacy rows pre-dating
   *  the KB-v1 backfill — callers should fall back to a
   *  `class_name` URL when null. */
  slug?: string | null;
  /** Per-category curated fields as a discriminated union. */
  summary: Summary;
}

export interface CohortRef {
  key: string;
  kind: string;
  label: string;
}

/** Detail shape returned by per-entry endpoints, with the full
 *  `metadata` blob retained for the detail page. The listing
 *  endpoint never returns this shape — only the slim
 *  `ReferenceEntry`. */
export interface ReferenceEntryDetail extends ReferenceEntry {
  metadata: Record<string, unknown>;
  /** Peer-group bucket key for stats lookup (server-derived). */
  peer_group?: string;
  /** Anchor's cohort memberships (server-stamped). */
  cohorts?: CohortRef[];
}

/** Map keyed by lowercased class_name → display_name. Retained for
 *  the legacy `prettyClass` callers in `event-summary.ts` and the
 *  journey/dashboard pages; new code should prefer
 *  [`ReferenceCatalog`] which keeps slug + summary attached. */
export type ReferenceMap = ReadonlyMap<string, string>;

/** Rich map keyed by lowercased class_name → full slim entry. */
export type ReferenceCatalog = ReadonlyMap<string, ReferenceEntry>;

/** One Map per category. Each Map is empty (not absent) on fetch
 *  failure so callers don't need a per-category presence check. */
export interface ReferenceLookup {
  vehicles: ReferenceMap;
  weapons: ReferenceMap;
  items: ReferenceMap;
  locations: ReferenceMap;
}

/** Empty lookup — safe default for callers before the fetch
 *  resolves. */
export const EMPTY_REFERENCE_LOOKUP: ReferenceLookup = {
  vehicles: new Map(),
  weapons: new Map(),
  items: new Map(),
  locations: new Map(),
};

/** Catalog form keyed by class_name → full slim entry, one per
 *  category. Populated alongside `ReferenceLookup` from the same
 *  fetch, so passing both around stays cheap. */
export interface ReferenceCatalogs {
  vehicles: ReferenceCatalog;
  weapons: ReferenceCatalog;
  items: ReferenceCatalog;
  locations: ReferenceCatalog;
}

/** Empty catalog set — safe default before the fetch resolves. */
export const EMPTY_REFERENCE_CATALOGS: ReferenceCatalogs = {
  vehicles: new Map(),
  weapons: new Map(),
  items: new Map(),
  locations: new Map(),
};

/** Bundle of both views produced by a single category fetch. */
export interface CategoryBundle {
  map: ReferenceMap;
  catalog: ReferenceCatalog;
  /** Deduplicated entry list — one per class_name. Use this rather
   *  than `Array.from(catalog.values())` because the catalog map
   *  may store each entry under multiple keys (class_name +
   *  display_name) so callers that want unique entries get
   *  duplicates from `.values()`. */
  list: readonly ReferenceEntry[];
}

export const EMPTY_CATEGORY_BUNDLE: CategoryBundle = {
  map: new Map(),
  catalog: new Map(),
  list: [],
};

/** Outcome of a `getEntityDetail` call. `not_found` lets the
 *  detail page render the dedicated 404 path; `error` lets the
 *  page distinguish transient backend trouble from a genuinely
 *  missing entity (the old single-`null` collapse rendered a
 *  permanent "not found" on a transient 503, which is misleading). */
export type EntityDetailOutcome =
  | { kind: 'ok'; entry: ReferenceEntryDetail }
  | { kind: 'not_found' }
  | { kind: 'error'; reason: string };

/**
 * Resolve a raw class identifier through a category Map; on miss,
 * fall through to the heuristic prettifier so the UI never renders
 * a bare underscored identifier. Pure — no fetches, no I/O.
 */
export function prettyClass(
  raw: string | null | undefined,
  map: ReferenceMap,
): string {
  if (!raw) return '';
  return map.get(raw.toLowerCase()) ?? toFriendlyName(raw);
}

/**
 * Variant / loaner suffixes appended to a base class name in some
 * event payloads but ABSENT from the wiki catalogue. Stripped as a
 * second lookup attempt so e.g. `ARGO_MOLE_Teach` (the tutorial
 * loaner) and `DRAK_Vulture_Teach` resolve to the catalogued
 * `ARGO_MOLE` / `DRAK_Vulture`. Lowercased; matched as a suffix.
 */
const VARIANT_SUFFIXES: readonly string[] = ['_teach', '_loaner'];

/**
 * Item class identifiers that are character-avatar parts, structural
 * placeholders, or engine defaults — never catalogued equipment. The
 * `attachment_received` stream is dominated by these (avatar assembly
 * on spawn), so without a filter the catalogue is consulted for
 * thousands of `Head_Eyelashes` / `Default` / `body_*` "items". Match
 * is case-insensitive; a hit means "render as plain text, never a
 * link". Conservative — only patterns confirmed against real logs.
 */
const NON_LINKABLE_ITEM_PATTERNS: readonly RegExp[] = [
  /^default(_|$)/i, // "Default", "Default_LensDisplay_PU"
  /^head_/i, // Head_Eyelashes, Head_Teeth, Head_Eyedetail
  /^body_/i, // body_01_noMagicPocket (corpse / avatar body)
  /^shared_scalp/i, // Shared_Scalp_Unified
  /^pu_protos/i, // PU_Protos_Head
  /^fp_visor$/i, // FP_Visor
  /^fps_default/i, // FPS_DefaultRadar_Lens
  /lensdisplay/i, // *_LensDisplay_* HUD glass
];

/**
 * True when an item class is avatar/structural noise rather than a
 * catalogued, linkable piece of equipment. Pure.
 */
export function isNonLinkableItemClass(classKey: string): boolean {
  return NON_LINKABLE_ITEM_PATTERNS.some((re) => re.test(classKey));
}

/**
 * Item *ports* that hold avatar customisation or structural sockets
 * rather than meaningful equipment (`Eyes_ItemPort`, `Hair_ItemPort`,
 * `Body_ItemPort`). Exposed so event-rendering surfaces can suppress
 * `attachment_received` noise by port. Pure.
 */
const COSMETIC_ITEM_PORTS: readonly RegExp[] = [
  /^(eyes|hair|eyelashes|eyebrow|beard|teeth|head|face)_itemport$/i,
  /^body_itemport$/i,
  /_scalp/i,
];

export function isCosmeticItemPort(port: string | null | undefined): boolean {
  if (!port) return false;
  return COSMETIC_ITEM_PORTS.some((re) => re.test(port));
}

interface ReferenceAliasIndex {
  byNormalized: ReadonlyMap<string, ReferenceEntry>;
}

const referenceAliasIndexes = new WeakMap<ReferenceCatalog, ReferenceAliasIndex>();

function getReferenceAliasIndex(catalog: ReferenceCatalog): ReferenceAliasIndex {
  const cached = referenceAliasIndexes.get(catalog);
  if (cached) return cached;

  const byNormalized = new Map<string, ReferenceEntry | null>();
  const seen = new Set<ReferenceEntry>();
  for (const entry of catalog.values()) {
    if (seen.has(entry)) continue;
    seen.add(entry);
    addAlias(byNormalized, entry, entry.class_name);
    addAlias(byNormalized, entry, entry.display_name);
    if (entry.category === 'vehicle') {
      addAlias(
        byNormalized,
        entry,
        entry.display_name.replace(/\s+limited$/i, ''),
      );
    }
  }

  const unique = new Map<string, ReferenceEntry>();
  for (const [key, entry] of byNormalized) {
    if (entry) unique.set(key, entry);
  }
  const index = { byNormalized: unique };
  referenceAliasIndexes.set(catalog, index);
  return index;
}

function addAlias(
  aliases: Map<string, ReferenceEntry | null>,
  entry: ReferenceEntry,
  value: string,
): void {
  const key = normalizeReferenceAlias(value);
  if (!key) return;
  if (!aliases.has(key)) {
    aliases.set(key, entry);
    return;
  }
  const existing = aliases.get(key);
  if (existing && existing !== entry) {
    aliases.set(key, null);
  }
}

function normalizeReferenceAlias(value: string): string {
  return value
    .normalize('NFKC')
    .replace(/\bjump\s+point\s+\d+\b/gi, 'jump point')
    .replace(/&/g, 'and')
    .replace(/[^a-z0-9]+/gi, '')
    .toLowerCase();
}

function aliasCandidates(
  category: ReferenceCategory,
  classKey: string,
): string[] {
  const candidates = [classKey];
  if (category !== 'location') return candidates;

  const route = classKey.match(
    /^\s*(.+?)\s*(?:↔|<->|->)\s*(.+?)\s*(?:·|-)?\s*jump\s+point(?:\s+\d+)?\s*$/i,
  );
  if (!route) return candidates;
  const from = route[1]?.trim();
  const to = route[2]?.trim();
  if (!from || !to) return candidates;
  candidates.push(`${from} - ${to} Jump Point`);
  candidates.push(`${from}-${to} Jump Point`);
  candidates.push(`${to} - ${from} Jump Point`);
  candidates.push(`${to}-${from} Jump Point`);
  return candidates;
}

function resolveByAlias(
  category: ReferenceCategory,
  classKey: string,
  catalog: ReferenceCatalog,
): ReferenceEntry | undefined {
  const index = getReferenceAliasIndex(catalog);
  for (const candidate of aliasCandidates(category, classKey)) {
    const key = normalizeReferenceAlias(candidate);
    const hit = index.byNormalized.get(key);
    if (hit) return hit;
  }
  return undefined;
}

/**
 * Resolve a raw class identifier to a catalog entry, applying two
 * fallbacks beyond the exact case-insensitive key:
 *   1. **Item noise filter** — avatar / structural item classes never
 *      resolve (so they render as plain text, never a misleading
 *      link). Only applies to `category === 'item'`.
 *   2. **Variant-suffix strip** — `_Teach` / `_loaner` loaner variants
 *      fall back to their base class (`ARGO_MOLE_Teach` → `ARGO_MOLE`).
 *      Applies to vehicle / weapon / item.
 *   3. **Display alias match** — normalized display names catch
 *      punctuation/spacing variants (`GrimHEX`, `P8 AR Rifle`) and
 *      compact labels (`85X` → `85X Limited`). Location jump-point
 *      route labels are matched in either direction.
 *
 * Returns undefined on miss.
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
  const alias = resolveByAlias(category, classKey, catalog);
  if (alias) return alias;
  // Locations don't carry loaner-style variant suffixes.
  if (category === 'location') return undefined;
  for (const suffix of VARIANT_SUFFIXES) {
    if (key.endsWith(suffix) && key.length > suffix.length) {
      const stripped = catalog.get(key.slice(0, -suffix.length));
      if (stripped) return stripped;
    }
  }
  return undefined;
}

// -- Location catalog types (Wave 1: catalog-driven hierarchy) ---------

/** Trimmed shape of a wiki location entry — only the fields we use
 *  for hierarchy resolution. Sourced from the raw wiki JSON which
 *  the server persists verbatim into `reference_registry.metadata`. */
export interface LocationEntry {
  /** Engine join key as the server stores it. Usually the wiki
   *  `slug` (e.g. `aberdeen-2`) since the wiki has no
   *  `class_name` field for locations. */
  classKey: string;
  /** Canonical display name (`"Aberdeen"`). */
  displayName: string;
  /** Parent system display from `star.name` (`"Stanton"`). */
  system: string | null;
  /** Parent body from `parent.name`. Null when the entry IS a
   *  planet or has no parent. */
  parent: string | null;
  /** Engine-internal joined short form from `tag.name`
   *  (`"Stanton1b"`). Primary match candidate against event
   *  payloads. */
  tag: string | null;
  /** URL slug (`"aberdeen-2"`). Match fallback. */
  slug: string | null;
  /** `type.classification` — `"Star"` / `"Planet"` / `"Moon"` /
   *  `"City"` / `"Station"` / `"Outpost"`. Drives display
   *  decisions (e.g. don't render `parent` for a planet). */
  classification: string | null;
}

/** Multi-index lookup over the location catalog. Several keys per
 *  entry so the parser can match by name, by engine tag, or by slug
 *  without knowing which form an event payload uses. */
export interface LocationCatalog {
  byName: ReadonlyMap<string, LocationEntry>;
  byTag: ReadonlyMap<string, LocationEntry>;
  bySlug: ReadonlyMap<string, LocationEntry>;
  display: ReferenceMap;
  /** Lookup of `class_name (lowercased) → slug`. Used by the
   *  HierarchicalBucketList to turn aggregate leaves into
   *  `/kb/location/{slug}` links when the leaf represents exactly
   *  one wiki-known location. Empty when slug backfill hasn't run. */
  slugByClass: ReadonlyMap<string, string>;
}

/** Empty catalog — safe default when the fetch fails or hasn't
 *  resolved yet. */
export const EMPTY_LOCATION_CATALOG: LocationCatalog = {
  byName: new Map(),
  byTag: new Map(),
  bySlug: new Map(),
  display: new Map(),
  slugByClass: new Map(),
};

/** Internal: listing-endpoint response shape used by the fetchers
 *  in `./reference`. Exported here so the server-side wrapper can
 *  type its JSON deserialisations against the same union the rest
 *  of the codebase reads. */
export interface ReferenceListResponse {
  entries: Array<{
    class_name: string;
    display_name: string;
    slug?: string | null;
    summary?: Summary;
  }>;
}
