#!/usr/bin/env node
/**
 * Operator tool: regenerate the committed reference snapshots by
 * pulling the consolidated catalogue from a running StarStats server.
 *
 * This does NOT run in CI — it is a manual step an operator runs when
 * the upstream catalogue has drifted enough to be worth re-seeding.
 * The package ships committed snapshots so it works without ever
 * running this.
 *
 * WHAT WE REDISTRIBUTE (the M10 legal posture): FACTS ONLY. Each
 * snapshot entry carries the engine `class_name`, the `display_name`,
 * the URL `slug`, and a `summary` reduced to a per-category allow-list
 * of FACTUAL fields (category discriminator, manufacturer, role,
 * taxonomy, plus any numeric spec values). Every free-text / prose /
 * HTML / `description` / long-`summary` field is DROPPED at extraction
 * time — those are the copyrightable parts, and we do not carry them.
 * Because the shipped snapshots are facts (names / specs / taxonomy)
 * plus CIG-derived data, attribution is CIG/RSI only (see
 * `src/attribution.ts`); no wiki prose travels with the data.
 *
 * FIRST-PARTY EXTENSIBILITY: each entry has room for a `custom` object
 * — our own fields, NOT sourced from any third party (see
 * `ConsolidatedEntry.custom` in `src/types.ts`). The generator never
 * writes `custom` (it only mirrors upstream facts); the loader passes
 * it through when a snapshot carries one, so a future first-party
 * enrichment can populate it without changing the loader.
 *
 * Usage:
 *   STARSTATS_API_URL=https://api.starstats.app \
 *     node scripts/generate.mjs --version 2026-07-21 --generated-at 2026-07-21T00:00:00Z
 *
 * Flags / env:
 *   STARSTATS_API_URL   base URL of the server, no trailing slash.
 *                       Defaults to https://api.starstats.app.
 *   --version <s>       snapshot set version tag (default: value of --generated-at).
 *   --generated-at <s>  timestamp string stamped into the manifest + files.
 *                       REQUIRED as an arg (or GENERATED_AT env) — this
 *                       script does not call Date.now(), so the caller
 *                       supplies the timestamp deterministically.
 *
 * Output: overwrites vehicle.json / weapon.json / item.json /
 * location.json (all MINIFIED, `reference-dump` format) + manifest.json
 * in ../snapshots.
 */

import { writeFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const DEFAULT_API_URL = 'https://api.starstats.app';
const CATEGORIES = ['vehicle', 'weapon', 'item', 'location'];

/** Provenance of the shipped snapshots — mirrors
 *  `attribution.DATA_PROVENANCE`. Facts + CIG-derived data, so CIG/RSI. */
const PROVENANCE = 'rsi-cig';
const SOURCE_IDS = ['rsi-cig'];

/**
 * Per-category allow-list of FACTUAL summary string keys. `category`
 * (the discriminator) is always kept. Any numeric or boolean value is
 * kept regardless of key ("any numeric spec fields"). Everything else
 * — unknown string keys, which is where prose / descriptions live — is
 * DROPPED. This is a positive allow-list on purpose: if upstream adds a
 * `description` field tomorrow, it is dropped by default rather than
 * silently redistributed.
 */
const FACT_KEYS = {
  vehicle: new Set(['manufacturer', 'role', 'hull_size', 'focus']),
  weapon: new Set(['manufacturer', 'size', 'damage_type', 'weapon_type']),
  item: new Set(['manufacturer', 'item_type', 'grade']),
  location: new Set([
    'system',
    'parent',
    'tag',
    'classification',
    // Wave 2 taxonomy (present only if upstream enriches the listing).
    'tier',
    'subtype',
    'operator',
    'faction',
  ]),
};

/** Object-valued factual keys kept verbatim (structured facts, not
 *  prose). `placement` is a discriminated spatial-relation record. */
const FACT_OBJECT_KEYS = {
  location: new Set(['placement']),
};

function parseArgs(argv) {
  const args = {};
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === '--version') args.version = argv[++i];
    else if (a === '--generated-at') args.generatedAt = argv[++i];
  }
  return args;
}

/**
 * Reduce a raw upstream `summary` to facts only. Keeps the `category`
 * discriminator, the per-category allow-listed string keys, any numeric
 * / boolean value, and allow-listed structured-fact objects. Drops
 * everything else (prose, HTML, descriptions).
 */
function extractFactualSummary(category, rawSummary) {
  const out = { category };
  if (!rawSummary || typeof rawSummary !== 'object') return out;
  const stringKeys = FACT_KEYS[category] ?? new Set();
  const objectKeys = FACT_OBJECT_KEYS[category] ?? new Set();
  for (const [k, v] of Object.entries(rawSummary)) {
    if (k === 'category') continue;
    if (v === null || v === undefined) continue;
    if (typeof v === 'number' || typeof v === 'boolean') {
      out[k] = v; // any numeric / boolean spec field
    } else if (typeof v === 'string') {
      if (stringKeys.has(k) && v.length > 0) out[k] = v;
      // Non-allow-listed strings are prose — dropped.
    } else if (typeof v === 'object' && objectKeys.has(k)) {
      out[k] = v; // structured fact (e.g. placement)
    }
  }
  return out;
}

async function fetchCategory(base, category) {
  const url = `${base}/v1/reference/${category}`;
  const resp = await fetch(url, { method: 'GET' });
  if (!resp.ok) {
    throw new Error(`GET ${url} -> ${resp.status} ${resp.statusText}`);
  }
  const body = await resp.json();
  return (body.entries ?? [])
    .filter((e) => e && e.class_name && e.display_name)
    .map((e) => ({
      class_name: e.class_name,
      display_name: e.display_name,
      slug: e.slug ?? null,
      summary: extractFactualSummary(category, e.summary),
    }));
}

async function main() {
  const args = parseArgs(process.argv.slice(2));
  const base = (process.env.STARSTATS_API_URL ?? DEFAULT_API_URL).replace(
    /\/$/,
    '',
  );
  const generatedAt = args.generatedAt ?? process.env.GENERATED_AT;
  const version = args.version ?? generatedAt;

  if (!generatedAt) {
    console.error(
      'ERROR: --generated-at <timestamp> (or GENERATED_AT env) is required.',
    );
    process.exit(2);
  }

  const here = path.dirname(fileURLToPath(import.meta.url));
  const snapshotsDir = path.resolve(here, '..', 'snapshots');

  const categories = {};
  for (const category of CATEGORIES) {
    const entries = await fetchCategory(base, category);
    const file = `${category}.json`;
    const payload = {
      category,
      format: 'reference-dump',
      version,
      generated_at: generatedAt,
      entries,
    };
    // MINIFIED — no pretty-printing. The loader imports these as JSON
    // modules, so whitespace is pure byte overhead on a 12k-entry file.
    await writeFile(
      path.join(snapshotsDir, file),
      JSON.stringify(payload) + '\n',
      'utf8',
    );
    categories[category] = {
      file,
      format: 'reference-dump',
      count: entries.length,
    };
    console.log(`wrote ${file} (${entries.length} entries)`);
  }

  const manifest = {
    version,
    generated_at: generatedAt,
    provenance: PROVENANCE,
    source_ids: SOURCE_IDS,
    categories,
  };
  await writeFile(
    path.join(snapshotsDir, 'manifest.json'),
    JSON.stringify(manifest) + '\n',
    'utf8',
  );
  console.log('wrote manifest.json');
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
