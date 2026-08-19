# reference-data

A **build-time static** package that consolidates the StarStats reference
catalogue (vehicle / weapon / item / location) from committed JSON
snapshots, plus the single source of truth for reference-data
**attribution**.

> **Additive, not a cutover.** This does **not** replace the runtime
> reference path (`apps/web/src/lib/reference.ts` → `/v1/reference/*` →
> `<EntityLink>`). It is a separate, statically-importable consumable.

## What it exports

```ts
import {
  loadConsolidatedCatalog, // () => ConsolidatedCatalog (memoised)
  lookupEntry,             // (classKey) => ConsolidatedEntry | undefined
  referenceManifest,       // () => ReferenceManifest
  SOURCES, DATA_PROVENANCE, SHIP_MATRIX_DISCLAIMER, WIKI_ATTRIBUTION,
} from 'reference-data';

// Attribution-only (no JSON snapshots pulled in):
import { SHIP_MATRIX_DISCLAIMER } from 'reference-data/attribution';
```

`loadConsolidatedCatalog()` returns every entry keyed by lower-cased
`class_name`, partitioned by category, alongside the manifest.

## Attribution (`src/attribution.ts`)

One place for reference-data credits:

- **Star Citizen Wiki** — CC BY-SA 4.0 (legally required attribution;
  do not strip while the data is wiki-derived).
- **RSI Ship Matrix** — © Cloud Imperium, first-party, shown as
  unofficial fan reference.

`DATA_PROVENANCE` (`'community-wiki'` today) and the `SOURCES` array are
structured so a future re-source to a **CIG/RSI-only** dataset is a
one-config change here — **deferred**, blocked on a first-party dataset.
Flipping it before the data is genuinely re-sourced would drop a
required CC BY-SA credit.

The verbatim Ship Matrix disclaimer lives here as
`SHIP_MATRIX_DISCLAIMER` and is imported by
`apps/web/src/components/kb/ShipMatrixDisclaimer.tsx`. The root `NOTICE`
file is the legal text of record and should reference the same sources.

## Snapshots (`snapshots/`)

| File            | Format               | Notes                                                        |
| --------------- | -------------------- | ------------------------------------------------------------ |
| `vehicle.json`  | `reference-dump`     | Small representative **seed** — regenerate for the full set. |
| `weapon.json`   | `reference-dump`     | Seed.                                                        |
| `item.json`     | `reference-dump`     | Seed.                                                        |
| `location.json` | `location-bootstrap` | Verbatim copy of the tray bootstrap snapshot (15 entries).   |
| `manifest.json` | —                    | Index: per-category file + format + count + provenance.      |

The location seed is the existing committed
`crates/starstats-client/assets/location_catalog.bootstrap.json`, copied
in verbatim so its shape (`taxonomy.tier`, `parent_body`, …) is reused
rather than re-transcribed. The loader normalises both formats via the
`format` the manifest records per category.

## Regenerating snapshots (operator tool)

Not run in CI. Dumps the full catalogue from a **running server**:

```bash
STARSTATS_API_URL=https://your-server.tld \
  node scripts/generate.mjs --version 2026-07-21 --generated-at 2026-07-21T00:00:00Z
```

- `STARSTATS_API_URL` — required, server base URL (no trailing slash).
- `--generated-at` — required (or `GENERATED_AT` env). Supplied by the
  caller so the run is deterministic; the script does not call
  `Date.now()`.
- `--version` — optional, defaults to `--generated-at`.

It overwrites all four `*.json` snapshots (a fresh dump writes
`location.json` in `reference-dump` format) + `manifest.json`. **Do not
commit multi-MB full dumps** — the committed state is intentionally
small seeds. Regenerate locally when you need the full catalogue.

## Tests

```bash
pnpm --filter reference-data test:run
```
