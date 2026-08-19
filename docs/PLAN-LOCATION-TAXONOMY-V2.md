# PLAN — Location taxonomy v2 (cross-stack)

**Status:** Draft for review. Builds on `docs/REFERENCE-CATALOG-HIERARCHY.md` (Wave 1).
**Owner:** TBD.
**Created:** 2026-05-22.

## Goal

Classify every location string that appears in game logs into a structured
`(tier, subtype, system, parent_body, spatial_relation)` tuple so that:

- The journey page's hierarchical rollups (Travel > destinations,
  Combat > hot zones) bucket by something richer than
  `system → body → place`.
- Players see meaningful sub-type chips ("Drug Lab", "Salvage Yard",
  "Naval Base") instead of "Outpost / unmapped".
- Both the tray (offline, no web) and the cloud sync server use the
  same classifier — so a player's local journey view matches what
  ships back to the cloud.

## Source-quality verdict (resolved)

We compared `api.star-citizen.wiki` (current source) vs `starcitizen.tools`
(MediaWiki) on 14 representative locations spanning every tier:

|                              | api.star-citizen.wiki | starcitizen.tools |
|---|---|---|
| Total location entries        | **1955**              | ~1073             |
| Has engine join key (`tag.name`) | **Yes** (`Stanton1b`, `Lorville`, …) | No (page-title only) |
| Has structured fields (jurisdiction, quantum_travel, size, amenities, …) | **Yes** (~25 fields) | No (wikitext infobox only) |
| Classification depth          | Coarse: Star / Planet / Moon / Settlement / Space Station / Landing Zone / Lagrange Point | **Rich**: 23 Landmark sub-types (Drug Lab, Salvage Yard, Distribution Center, Racetrack, FOB, …), Naval bases, Flotillas |
| Spatial-relation tags         | Single `parent.name`  | **`On <Body>` / `Orbits <Body>` / `Lagrange Point Lx <Body>` / `Sunward from <Body>`** |
| Faction / operator tagging    | `affiliation` only    | **Faction + operator categories** (Hurston Dynamics, Crusader Industries, Shubin Interstellar, Nine Tails, XenoThreat, Rough & Ready) |
| Klescher Rehabilitation Facility coverage | Yes | No (disambig page) |
| API stability                 | Stable JSON, paginated | MediaWiki API, redirects/disambigs need handling |

**Decision: keep api.star-citizen.wiki as primary; add starcitizen.tools
as a secondary enrichment source.** Join on `slug` (primary) ↔
normalized page title (secondary) with name as the tiebreaker.

## Target classification

```rust
struct LocationClassification {
    // Display
    display_name:   String,
    slug:           String,                       // primary key from api.star-citizen.wiki

    // Tier — one of 8 mutually-exclusive buckets
    tier:           LocationTier,
    // Sub-type — narrows the tier (Drug Lab inside Landmark, Rest Stop inside Space Station, …)
    subtype:        Option<LocationSubtype>,

    // Hierarchy — system / body chain
    system:         Option<String>,               // "Stanton", "Pyro", "Tiber"
    parent_body:    Option<String>,               // immediate parent ("Hurston", "Crusader", …)

    // Spatial-relation tag — from starcitizen.tools
    placement:      Option<Placement>,            // On(Daymar) / Orbits(Yela) / Lagrange(L1, Hurston) / Sunward(Hurston)

    // Engine join keys
    engine_tag:     Option<String>,               // "Stanton1b" — from tag.name
    engine_class:   Option<String>,               // raw OOC_Stanton_3_Lorville-style name if known

    // Affiliations
    jurisdiction:   Option<String>,               // "UEE", "Unclaimed", "Banu Protectorate"
    operator:       Option<String>,               // "Hurston Dynamics", "Shubin Interstellar"
    faction:        Option<String>,               // "Nine Tails", "XenoThreat", "Rough & Ready"

    // Provenance
    source_primary:   Provenance,                 // api.star-citizen.wiki — always
    source_enrichment: Option<Provenance>,        // starcitizen.tools — when matched
}

enum LocationTier {
    System,                // "Stanton system", "Pyro system"
    AstronomicalObject,    // planets, moons, asteroid belts, planetoids, nebulae, stars
    LandingZone,           // hero cities
    SpaceStation,          // orbital man-made
    Landmark,              // named on-body developments
    Flotilla,              // gathered fleets
    NavalBase,             // military installations
    AnonymousPoi,          // dynamic / procedural — no wiki entry
}

enum LocationSubtype {
    // AstronomicalObject
    Star, Planet, Moon, Planetoid, AsteroidBelt, Nebula,
    // SpaceStation
    OrbitalStation, RestStop, GatewayStation, OrbitalLaserPlatform, AsteroidBase,
    SealedSettlement,  // dual-tagged on starcitizen.tools (e.g. Grim HEX)
    // LandingZone
    City,
    // Landmark
    Outpost, Settlement, Spaceport, SalvageYard, DrugLab, DistributionCenter,
    Racetrack, ConventionCenter, Shelter, ForwardOperatingBase, Hospital,
    Market, Bar, Restaurant, Museum, ApartmentHab, Lounge, CommercialBuilding,
    PlanetaryAlignmentFacility, Geomorphology,
    // AnonymousPoi (engine-only, no wiki entry)
    CommArray, JumpPoint, CrashSite, Cave, Bunker, DerelictShip,
}

enum Placement {
    OnBody(String),                    // "On Daymar"
    OrbitsBody(String),                // "Orbits Yela"
    LagrangePoint { lagrange: u8, body: String }, // L1..L5, e.g. ("L1", "Hurston")
    SunwardFrom(String),               // "Sunward from Hurston"
    AngleFrom { degrees: i16, body: String }, // "-60° from Monox"
}
```

## Architecture

```
                                                      ┌──────────────────┐
        ┌────────────────────────────────────┐        │ api.star-citizen │
        │  starstats-server (cloud-sync)     │◀──────│  .wiki           │
        │                                    │  daily │   /api/locations │
        │  • reference_data.rs    (cron)     │  cron  │   (1955 entries) │
        │  • reference_store.rs   (CRUD)     │        └──────────────────┘
        │  • locations.rs         (enrich)   │
        │  • backfill_locations.rs (one-shot)│        ┌──────────────────┐
        │                                    │◀──────│  starcitizen     │
        │  PG: reference_registry            │  daily │  .tools          │
        │      + location_taxonomy_v2 (cols) │  cron  │  MediaWiki API   │
        └────────────────────────────────────┘        │  (categories +   │
              │                ▲                      │   page tags)     │
              │ /v1/reference  │ /v1/ingest           └──────────────────┘
              ▼                │
        ┌──────────────────────────────────┐
        │  starstats-core (shared)         │
        │                                  │
        │  • location_classifier.rs        │  ← NEW
        │     LocationClassifier::classify │
        │  • location_catalog.rs           │  ← NEW
        │     loads, indexes by tag/name   │
        │  • location_taxonomy.rs          │  ← NEW
        │     LocationTier / Subtype enums │
        └──────────────────────────────────┘
              │                ▲
              │                │
   ┌──────────┴────────┐   ┌───┴────────────────┐
   │ starstats-client  │   │ apps/web (Next.js) │
   │  (Tauri tray)     │   │                    │
   │                   │   │ • reference.ts     │
   │ • gamelog.rs      │   │ • class-name-parts │
   │ • commands.rs     │   │   (thin wrapper    │
   │ • storage.rs      │   │   over server      │
   │   (cached         │   │   classification)  │
   │   catalog JSON)   │   │ • EntityLink.tsx   │
   └───────────────────┘   └────────────────────┘
```

Key invariants:

1. **Classifier lives in `starstats-core`.** Identical Rust implementation runs in
   tray (offline first; ships a fallback snapshot) and server (cloud
   ingest path). Web renders server-classified data plus a TS port for
   offline-mode-on-the-web (long-tail).
2. **Catalog is the join.** Both data sources land in the
   `reference_registry` table; enrichment adds columns (or a `taxonomy_v2`
   JSONB). The classifier consults the indexed catalog by `engine_tag`
   first, then by normalized name/slug.
3. **Classification is deterministic and cache-friendly.** No network
   calls in the hot path — everything joins against the in-memory
   catalog loaded from DB.

## Phases (per dev-workflow-protocol Rule 4)

### Phase 0 — Research (DONE)

- Verified source-quality verdict above.
- Confirmed `api.star-citizen.wiki` exposes 1955 entries, structured shape, engine tag.
- Confirmed `starcitizen.tools` has 7 taxonomic tiers + 23 Landmark sub-buckets + spatial-relation tags.
- Audited current code: `class-name-parts.ts` (727 lines, hardcoded dicts + SYNTHETIC_MATCHERS), `reference_data.rs`, `reference_store.rs`, `crates/starstats-core/src/{events,parser,metadata}.rs`, `crates/starstats-client/src/gamelog.rs`.
- See [[sc-wiki-location-taxonomy]] memory for the rich-taxonomy reference.

### Phase 1 — DB schema + enrichment cron (server-only)

Land the data first; nothing else can move without it.

| File | Change | Size |
|---|---|---|
| `crates/starstats-server/migrations/0039_location_taxonomy_v2.sql` | NEW. Adds nullable columns to `reference_registry`: `tier TEXT`, `subtype TEXT`, `taxonomy_v2 JSONB` (the latter holds placement + operator + faction; display-only). B-tree indexes on `(category, tier)` and `(category, subtype)` for the journey-page filter hot path. CHECK on `tier` allow-list. ADDITIVE only per migration convention. | small |
| `crates/starstats-server/src/reference_data.rs` | NEW `ToolsWikiEnrichmentClient` impl alongside `WikiReferenceClient`. Fetches `Category:Locations` (+ Landmarks, Astronomical objects, etc.) via MediaWiki API with continuation, then per-page categories. Normalizes title → slug (lower-kebab, strip parentheticals). | medium |
| `crates/starstats-server/src/reference_store.rs` | Add `upsert_location_taxonomy(slug, taxonomy)` method. Idempotent on slug. | small |
| `crates/starstats-server/src/main.rs` | Schedule the new enrichment cron alongside the existing wiki cron — daily, offset by 1h so they don't both peak at midnight. | small |
| `crates/starstats-server/src/reference_routes.rs` | Add the new fields to the `/v1/reference/location` response shape. | small |
| `crates/starstats-server/tests/...` | Integration test: stubbed enrichment client returns known categories; assert `tier='Landmark', subtype='DrugLab', placement={On, Daymar}` for a `Jumptown`-shaped row. | medium |

**Joined on slug**, with title-normalization fallbacks documented inline.
Klescher-style disambig pages handled via a hardcoded override list
(keep ≤ 20 entries; if it grows, that's a signal to revisit).

Verification:
- New API response includes `tier`, `subtype`, `placement`, `operator`, `faction` for all locations that match.
- Diff old vs new `/v1/reference/location` shows ~600+ locations gain `tier='Landmark'` with sub-type, ~50 gain `tier='NavalBase'` or `Flotilla` re-classification.
- `cargo test -p starstats-server` passes.

### Phase 2 — Shared classifier in starstats-core

| File | Change | Size |
|---|---|---|
| `crates/starstats-core/src/location_taxonomy.rs` | NEW. `LocationTier`, `LocationSubtype`, `Placement` enums + serde. Compile-time exhaustive. | medium |
| `crates/starstats-core/src/location_catalog.rs` | NEW. `LocationCatalog` struct with `by_engine_tag`, `by_slug`, `by_normalized_name` indices. `from_reference_entries(Vec<ReferenceEntry>)` builds it. | medium |
| `crates/starstats-core/src/location_classifier.rs` | NEW. `classify(raw: &str, catalog: &LocationCatalog) -> LocationClassification`. Port `class-name-parts.ts`'s Tier 0 (catalog) + SYNTHETIC_MATCHERS (Jump Points, Comm Arrays) into Rust. Keeps the engine-only short-codes (`HUR_L1`, `Shubin_*`, `HDMS_*`) as last-resort fallbacks. | large |
| `crates/starstats-core/src/lib.rs` | Re-export new modules. | trivial |
| `crates/starstats-core/tests/location_classifier.rs` | NEW. Table-driven test against a frozen catalog fixture: every existing entry in `class-name-parts.ts` test cases gets a Rust mirror, plus new cases for Landmark sub-types and Placement tags. ~80 cases. | large |

Verification:
- `cargo test -p starstats-core` — 100% of TS test cases pass under Rust.
- Bench: classify 10k locations < 50ms on a warm catalog (target: 10x faster than today's TS path since no regex backtracking).

### Phase 3 — Server classifier cache (revised — derive, don't denormalize)

Reading the existing schema revealed `events.payload JSONB` + an existing
`locations.rs` module that derives location info at query time.
Denormalizing classification onto event rows would require a migration,
a backfill, AND would drift whenever the catalog updates. Instead:

| File | Change | Size |
|---|---|---|
| `crates/starstats-server/src/location_catalog_cache.rs` | NEW. `LocationCatalogCache` — `Arc<RwLock<Arc<LocationCatalog>>>` plus a `refresh()` method that rebuilds from `ReferenceStore::list_category(Location)`. Cheap snapshot reads via `cache.snapshot()` returning an `Arc<LocationCatalog>`. | medium |
| `crates/starstats-server/src/reference_data.rs` | NEW pub helper `entry_to_catalog_entry(ReferenceEntry) -> LocationCatalogEntry` translating wiki metadata + taxonomy_v2 into the shared core shape. | small |
| `crates/starstats-server/src/main.rs` | Build the cache at startup; pass `cache.refresh()` callbacks into both crons so the cache stays warm. | small |
| `crates/starstats-server/src/locations.rs` | New helper `classify_event_payload(payload, &LocationCatalog) -> Option<LocationClassification>` extracting the right token from `QuantumTargetSelected.destination`, `JoinPu.location_id`, etc. and calling the core classifier. | medium |

Phase 6 (backfill) is therefore deleted from this plan — there's no
denormalized state to backfill. Any future filter UI on `location_tier`
joins reference_registry at query time.

Verification:
- New event ingested with `destination="OOC_Stanton_3_Lorville"` lands with `location_tier='LandingZone', location_subtype='City', location_system='Stanton', location_parent_body='Hurston'`.
- Existing journey query returns the new fields when called with a recent date range.

### Phase 4 — Tray integration (starstats-client)

| File | Change | Size |
|---|---|---|
| `crates/starstats-client/src/storage.rs` | Add `location_catalog.json` cached snapshot file alongside existing cached configs. Ship a bundled fallback at `crates/starstats-client/assets/location_catalog.bootstrap.json` so the tray works on first run with no network. | small |
| `crates/starstats-client/src/sync.rs` | On startup (and on a 24h timer), fetch `/v1/reference/location` and replace the local snapshot. Surface a `LocationCatalogReady` event to the UI. | medium |
| `crates/starstats-client/src/gamelog.rs` | After parsing an event, call the shared classifier to attach a `LocationClassification` to the event payload. | small |
| `crates/starstats-client/src/commands.rs` | New Tauri command `get_location_classification(raw: String) -> LocationClassification` for ad-hoc UI lookups. | small |
| `apps/desktop/src/components/journey/*` (Tauri UI) | Same UI patterns as web — use the Tauri-provided classification rather than re-parsing. | medium |

Verification:
- Tray works offline on first launch using bundled snapshot.
- Snapshot refresh on second launch picks up new locations added since release.
- Killing the tray's local DB and re-importing logs reproduces identical classifications to a server-side round-trip.

### Phase 5 — Web integration (apps/web)

| File | Change | Size |
|---|---|---|
| `apps/web/src/lib/reference-types.ts` | Extend `LocationEntry` with `tier`, `subtype`, `placement`, `operator`, `faction`. | small |
| `apps/web/src/lib/reference.ts` | `getLocationCatalog()` returns enriched entries; multi-index updated to include `by_tier`, `by_subtype` for the journey-page filter UI. | small |
| `apps/web/src/lib/class-name-parts.ts` | **Thin** down from 727 lines: keep `stripAndSplit`, `isNonDestination`, `parseWeaponClass`, `parseItemClass`. Replace `parseLocationClass` with a 30-line wrapper that consults the catalog and falls back to the same SYNTHETIC_MATCHERS / KNOWN_BODIES dict (kept for engine-only short-codes — that's not the wiki's job). | medium |
| `apps/web/src/components/kb/EntityLink.tsx` | When `category='location'`, render tier chip + subtype tag. | small |
| `apps/web/src/components/kb/EntityHoverCard.tsx` | Show `placement` (e.g. "Orbits Yela"), `operator`, `faction` on hover. | small |
| `apps/web/src/components/journey/HierarchicalBucketList.tsx` | New grouping dimension: tier OR subtype. Toggle in UI. Preserve existing system/body grouping. | medium |
| `apps/web/src/app/journey/_components/TypesTab.tsx` | Filter bar gains tier+subtype facets. | medium |
| `apps/web/src/lib/event-summary.ts` / `event-summary-react.tsx` | Event timeline lines use the classifier output (no parsing in the renderer). | small |

Verification:
- Existing journey-page screenshots remain consistent at system/body roll-up level.
- New "by tier" toggle groups locations as Astronomical / Stations / Landing zones / Landmarks / etc.
- `EntityLink` for `Jumptown` renders "Landmark · Drug Lab · on Daymar".

### Phase 6 — DELETED (no backfill needed)

The Phase-3 revision (derive, don't denormalize) eliminates the
need for a backfill: classification is computed at query time
against the current catalog snapshot, so old events
automatically pick up new classifications as the catalog evolves.

### Phases 7-9 — Reviews + finalize (per Rule 4)

In parallel after phases 1-6 complete:

- **Phase 7 (fact-check)**: trace `OOC_Stanton_3_Lorville` from a raw log line through ingest → DB → /v1/me/events → journey-page rollup. Assert tier/subtype/system/parent_body are correct at every layer.
- **Phase 8 (security/quality)**: audit the new cron's MediaWiki fetch for SSRF (we only hit a hardcoded host); audit JSONB persistence for indexable depth; audit admin endpoint AuthN/Z.
- **Phase 9 (readability)**: classifier module size, enum exhaustiveness, doc comments on every public type.

Then **Phase 10 (synthesis)**: fix anything raised, re-run full Rust + TS test suites.
**Phase 11 (finalize)**: rebuild containers, smoke-test journey page on staging, update CHANGELOG.md.

## Test plan

### Unit
- `crates/starstats-core/tests/location_classifier.rs` — 80+ table-driven cases. Goldmaster: capture today's `class-name-parts.ts` output for the existing fixture log, freeze as expected output; new classifier must reproduce every existing classification plus add tier/subtype.
- `apps/web/src/lib/__tests__/class-name-parts.test.ts` — existing tests stay green (parser is now a wrapper).
- `crates/starstats-server/tests/reference_enrichment.rs` — stubbed MediaWiki responses produce expected taxonomy.

### Integration
- `crates/starstats-server/tests/ingest_classification.rs` — full ingest → classify → query roundtrip.
- `apps/web/src/app/journey/__tests__/types-tab.test.tsx` — new filter UI behaves correctly across catalog states (loaded / empty / partial).

### Goldmaster
- Capture the current journey-page render for the bundled sample log; assert by-system grouping unchanged after migration. New chips/badges are additive only.

### Manual smoke
- Tray offline (no network) on first launch — locations still classify via bundled snapshot.
- Web UI: Lorville renders "Landing Zone · City · on Hurston"; Jumptown renders "Landmark · Drug Lab · on Daymar"; Ruin Station renders "Space Station · — · orbits Terminus"; Bacchus Flotilla renders "Flotilla · Banu Flotilla · orbits Bacchus A".

## Risks + mitigations

| Risk | Mitigation |
|---|---|
| MediaWiki API rate-limits the daily cron | Concurrency cap (4), respect `Retry-After`, fail-open (skip enrichment, keep primary fields). |
| starcitizen.tools page title ≠ api.star-citizen.wiki slug (disambig, redirects) | Maintain a hardcoded override map for the long tail (~20 entries). Log unmatched primary rows for human review. |
| Engine string in logs has no catalog match (procedural POI) | Tier=`AnonymousPoi`, subtype derived from string pattern (CommArray / JumpPoint / CrashSite / Cave / Bunker / DerelictShip). Same SYNTHETIC_MATCHERS as today, just lifted into Rust. |
| Tray snapshot drifts from server catalog | 24h timer + on-launch refresh. Show a "last updated" timestamp in About. |
| Backfill on prod DB is slow | Batched 1000-row job with `LIMIT`/`OFFSET` and a `WHERE classification IS NULL` resumability check. |
| EAC interaction (tray reads game logs) | No new file access patterns; classifier reads in-memory data only. See `EAC-SAFETY.md`. |

## Out of scope

- Combining wiki data with player-supplied custom location names. If we need it later, layer a `location_overrides` table on top.
- Localization beyond English.
- Real-time enrichment — daily cron is sufficient given how rarely the wiki adds new locations.
- Cross-version analytics (does Jumptown's classification differ between SC 4.7 and 4.8?). Out of scope; if needed later, snapshot per-patch.

## Resolved decisions

- **DB column shape:** per-field nullable columns for the two high-cardinality filter dimensions (`tier`, `subtype`); single `taxonomy_v2 JSONB` column for `placement` + `operator` + `faction`. Rationale: the journey page's hot path filters/groups by tier and subtype, so they need real B-tree indexes; placement and the affiliations are display-only and never appear in WHERE clauses.
- **Tray snapshot format:** plain JSON. 1955 entries × ~400 bytes ≈ 800 KB gzipped — well within the Tauri bundle budget, and avoids a CBOR/msgpack dependency in `starstats-client`.
- **Phase ordering:** Phase 5 (web) ships before Phase 6 (backfill). New events validate the classifier end-to-end through the cloud path; once that's green, the backfill cleans up history without risk of re-running against a buggy classifier.
