/**
 * Shared type surface for the `reference-data` package.
 *
 * These mirror (a subset of) the runtime reference types in
 * `apps/web/src/lib/reference-types.ts`, deliberately re-declared
 * here rather than imported: a workspace package must not depend on
 * an app. Keep the two in sync when the category vocabulary changes.
 */

/** The four reference categories, matching the runtime `/v1/reference/{category}` set. */
export type ReferenceCategory = 'vehicle' | 'weapon' | 'item' | 'location';

export const CATEGORIES: readonly ReferenceCategory[] = [
  'vehicle',
  'weapon',
  'item',
  'location',
];

/**
 * How a committed snapshot file is shaped, so the loader knows how to
 * normalise it. `reference-dump` is what the generator writes from a
 * live `/v1/reference/{category}` pull. `location-bootstrap` is the
 * reused tray bootstrap snapshot
 * (`crates/starstats-client/assets/location_catalog.bootstrap.json`),
 * copied in verbatim as the location seed — so it keeps its own shape
 * and the manifest records how to read it.
 */
export type SnapshotFormat = 'reference-dump' | 'location-bootstrap';

/**
 * Where the shipped reference data comes from. Since the M10 cutover
 * the catalogue redistributes FACTS ONLY (names / specs / taxonomy)
 * plus CIG-derived data — no wiki prose — so provenance is `'rsi-cig'`.
 * `'community-wiki'` is retained only so older snapshots still type.
 */
export type DataProvenance = 'community-wiki' | 'rsi-cig';

/**
 * Per-entry FACTUAL summary. Deliberately an open record rather than
 * the app's discriminated `Summary` union — a workspace package must
 * not depend on an app. It always carries the `category` discriminator
 * plus the per-category factual fields the generator allow-lists
 * (manufacturer / role / taxonomy / numeric specs). Never prose. The
 * web app casts this to its `Summary` type at the consumption boundary.
 */
export interface ReferenceSummary {
  category: ReferenceCategory;
  [field: string]: unknown;
}

/**
 * One normalised catalogue entry. The consolidated view every consumer
 * of this package reads: `class_name → { display_name, category, slug,
 * summary, tier? }`, plus the lower-cased `classKey` used for lookups.
 */
export interface ConsolidatedEntry {
  /** Lower-cased `className`, the lookup key (mirrors the runtime
   *  catalogue's case-insensitive keying). */
  classKey: string;
  /** Engine class identifier as stored upstream (`AEGS_Avenger_Titan`). */
  className: string;
  /** Human-friendly name (`Avenger Titan`). */
  displayName: string;
  category: ReferenceCategory;
  /** URL-safe canonical id; null on legacy rows without a backfilled slug. */
  slug: string | null;
  /** Per-entry factual summary (category discriminator + facts). Always
   *  present for `reference-dump` snapshots; the web catalog builder
   *  reads it straight through as its `Summary`. */
  summary: ReferenceSummary;
  /** Location-only coarse tier (`astronomical_object`, `landing_zone`, …).
   *  Absent for vehicle / weapon / item entries. */
  tier?: string;
  /** First-party fields NOT sourced from any third party — reserved for
   *  future StarStats-authored enrichment. Absent unless a snapshot
   *  carries one; the generator never writes it (it only mirrors
   *  upstream facts). Documented open shape so the loader passes it
   *  through untouched. */
  custom?: Record<string, unknown>;
}

/** Per-category manifest record. */
export interface ManifestCategory {
  file: string;
  format: SnapshotFormat;
  count: number;
}

/**
 * Snapshot manifest — the index the loader reads to discover which
 * file backs each category and how to parse it. A future flip to a
 * first-party dataset changes the files + `provenance` here; consumers
 * don't change.
 */
export interface ReferenceManifest {
  /** Snapshot set version (operator-supplied, e.g. a date or dump tag). */
  version: string;
  /** ISO-ish timestamp the snapshot set was generated (operator-supplied). */
  generated_at: string;
  /** Provenance flag — mirrors `attribution.DATA_PROVENANCE`. */
  provenance: DataProvenance;
  /** Attribution source ids (see `attribution.SOURCES`) backing this set. */
  source_ids: string[];
  categories: Record<ReferenceCategory, ManifestCategory>;
}

/** The consolidated catalogue produced by `loadConsolidatedCatalog`. */
export interface ConsolidatedCatalog {
  /** Every entry, one per (category, class_name). */
  entries: readonly ConsolidatedEntry[];
  /** Lookup keyed by lower-cased class_name. When the same class_name
   *  appears in two categories, the first-loaded wins and later ones
   *  are still present in `entries` and `byCategory`. */
  byClassName: ReadonlyMap<string, ConsolidatedEntry>;
  /** Entries partitioned by category. */
  byCategory: Record<ReferenceCategory, readonly ConsolidatedEntry[]>;
  manifest: ReferenceManifest;
}
