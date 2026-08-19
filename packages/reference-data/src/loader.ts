/**
 * Build-time loader for the consolidated reference catalogue.
 *
 * Reads the committed JSON snapshots (see `../snapshots/`) and
 * normalises the two on-disk shapes — the generator's `reference-dump`
 * format and the reused `location-bootstrap` format — into one flat
 * `ConsolidatedCatalog` keyed by class_name.
 *
 * Pure + synchronous: the snapshots are bundled at build time, so there
 * is no I/O and no network. This is the STATIC counterpart to the
 * runtime path in `apps/web/src/lib/reference.ts` (which fetches
 * `/v1/reference/*` live). This package is ADDITIVE — it does not
 * replace that runtime path.
 */

import manifestJson from '../snapshots/manifest.json';
import vehicleJson from '../snapshots/vehicle.json';
import weaponJson from '../snapshots/weapon.json';
import itemJson from '../snapshots/item.json';
import locationJson from '../snapshots/location.json';

import {
  CATEGORIES,
  type ConsolidatedCatalog,
  type ConsolidatedEntry,
  type ReferenceCategory,
  type ReferenceManifest,
  type ReferenceSummary,
} from './types';

// ---- Raw on-disk shapes (what the JSON files actually contain) -------

/** An entry in a `reference-dump` snapshot (generator output + seeds). */
interface RawDumpEntry {
  class_name: string;
  display_name: string;
  slug?: string | null;
  summary?: unknown;
  custom?: Record<string, unknown>;
}

/** An entry in the reused `location-bootstrap` snapshot. */
interface RawBootstrapEntry {
  class_name: string;
  display_name: string;
  slug?: string | null;
  taxonomy?: { tier?: string; subtype?: string };
}

interface RawSnapshot {
  entries?: Array<RawDumpEntry & RawBootstrapEntry>;
}

// The imported JSON is typed structurally by tsc; cast to our raw
// shapes through `unknown` so a snapshot edit can't silently widen the
// loader's view of the data.
const RAW_SNAPSHOTS: Record<ReferenceCategory, RawSnapshot> = {
  vehicle: vehicleJson as unknown as RawSnapshot,
  weapon: weaponJson as unknown as RawSnapshot,
  item: itemJson as unknown as RawSnapshot,
  location: locationJson as unknown as RawSnapshot,
};

const MANIFEST = manifestJson as unknown as ReferenceManifest;

/** Normalise one snapshot's entries for a category into consolidated entries. */
function normaliseCategory(
  category: ReferenceCategory,
  snapshot: RawSnapshot,
): ConsolidatedEntry[] {
  const format = MANIFEST.categories[category]?.format;
  const out: ConsolidatedEntry[] = [];
  for (const raw of snapshot.entries ?? []) {
    if (!raw.class_name || !raw.display_name) continue;
    // Normalise the factual summary. `reference-dump` snapshots ship a
    // per-entry facts object; guarantee the `category` discriminator so
    // consumers can narrow on it even for a malformed / summary-less row.
    const rawSummary =
      raw.summary && typeof raw.summary === 'object'
        ? (raw.summary as Record<string, unknown>)
        : {};
    const summary: ReferenceSummary = { ...rawSummary, category };
    const entry: ConsolidatedEntry = {
      classKey: raw.class_name.toLowerCase(),
      className: raw.class_name,
      displayName: raw.display_name,
      category,
      slug: raw.slug ?? null,
      summary,
    };
    // First-party fields, if a snapshot carries any — passed through
    // untouched (the generator never writes these).
    if (raw.custom && typeof raw.custom === 'object') {
      entry.custom = raw.custom;
    }
    // Tier: taxonomy.tier from the legacy location-bootstrap shape, or a
    // `tier` fact on a reference-dump location summary.
    if (format === 'location-bootstrap' && raw.taxonomy?.tier) {
      entry.tier = raw.taxonomy.tier;
    } else if (typeof summary.tier === 'string') {
      entry.tier = summary.tier;
    }
    out.push(entry);
  }
  return out;
}

let cached: ConsolidatedCatalog | null = null;

/**
 * Load the consolidated reference catalogue from the committed
 * snapshots. Memoised — the snapshots are immutable at runtime, so the
 * first call builds the maps and every later call reuses them.
 */
export function loadConsolidatedCatalog(): ConsolidatedCatalog {
  if (cached) return cached;

  const byCategory = {} as Record<ReferenceCategory, ConsolidatedEntry[]>;
  const entries: ConsolidatedEntry[] = [];
  const byClassName = new Map<string, ConsolidatedEntry>();

  for (const category of CATEGORIES) {
    const normalised = normaliseCategory(category, RAW_SNAPSHOTS[category]);
    byCategory[category] = normalised;
    for (const entry of normalised) {
      entries.push(entry);
      // First writer wins on cross-category class_name collisions so
      // lookups are stable; the entry still appears in `byCategory`.
      if (!byClassName.has(entry.classKey)) {
        byClassName.set(entry.classKey, entry);
      }
    }
  }

  cached = {
    entries,
    byClassName,
    byCategory,
    manifest: MANIFEST,
  };
  return cached;
}

/**
 * Resolve a raw class identifier to its consolidated entry (case-
 * insensitive). Pure convenience over `loadConsolidatedCatalog()`.
 */
export function lookupEntry(
  classKey: string | null | undefined,
): ConsolidatedEntry | undefined {
  if (!classKey) return undefined;
  return loadConsolidatedCatalog().byClassName.get(classKey.toLowerCase());
}

/** The snapshot manifest (version, provenance, per-category counts). */
export function referenceManifest(): ReferenceManifest {
  return MANIFEST;
}

/**
 * Reset the memoised catalogue. Test-only — the module-level cache
 * otherwise leaks across test cases.
 * @internal
 */
export function __resetConsolidatedCatalogForTests(): void {
  cached = null;
}
