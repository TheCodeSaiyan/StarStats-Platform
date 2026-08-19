# Reference catalog: catalog-driven hierarchy

**Status:** Wave 1 in progress (web-side). Wave 2 deferred.
**Owners:** StarStats client + server.

## Problem

The journey page's location rollups (Travel > destinations, Combat >
hot zones) hierarchize raw engine destinations into system → body →
place. Until now that hierarchy lived in three hardcoded TS
dictionaries (`KNOWN_SYSTEMS`, `KNOWN_BODIES`, `KNOWN_PLACES` in
`apps/web/src/lib/class-name-parts.ts`).

Drawbacks of the hardcoded approach:

1. **Manual maintenance.** Every new Star Citizen location requires
   a code change + redeploy. SC drops new places every patch.
2. **Drift from canonical source.** The wiki is the community's
   source of truth for SC locations. Hardcoded TS names drift from
   wiki spellings.
3. **Coverage gaps.** With 200+ known SC locations and 27 in the TS
   dicts, anything not hand-mapped lands in `Other / unmapped`.

## Insight: the data is already in the database

The daily wiki cron (`crates/starstats-server/src/reference_data.rs`)
calls `parse_category_page` for each category. That function
**already** persists the full wiki JSON (minus `id`/`created_at`/
`updated_at`/`version`) into the `reference_registry.metadata` JSONB
column. We've been throwing away the hierarchy on the web side.

A real wiki location entry looks like:

```json
{
  "uuid": "f8f07f5b-1c0e-47c9-aa50-46963065bf18",
  "slug": "aberdeen-2",
  "name": "Aberdeen",
  "system": "Stanton System",
  "star":   { "name": "Stanton", "type_name": "Star" },
  "parent": { "name": "Hurston", "type_name": "Planet" },
  "type":   { "name": "Moon", "classification": "Moon" },
  "tag":    { "name": "Stanton1b" },
  "designation": "Stanton Ib",
  ...
}
```

The fields we care about for hierarchy:

| Field | Purpose |
|---|---|
| `star.name` | Canonical system display (`"Stanton"`) |
| `parent.name` | Canonical parent body (`"Hurston"`, `null` for planets) |
| `name` | Canonical place display (`"Aberdeen"`) |
| `tag.name` | Engine-internal joined form (`"Stanton1b"`) — primary match candidate |
| `slug` | URL-safe form, useful as a fallback match key |
| `type.classification` | `Star` / `Planet` / `Moon` / `City` / `Station` / `Outpost` |

## Architecture (Wave 1)

```
Wiki API: GET /api/locations  (paginated, daily cron)
                              |
                              v
reference_registry table
  category=location | class_name=<slug> | metadata=<full wiki JSON>
                              |
                              | GET /v1/reference/location
                              v
Web client: getLocationCatalog()
  byName: Map<lowercase name -> LocationEntry>
  byTag:  Map<lowercase tag.name -> LocationEntry>
  bySlug: Map<lowercase slug -> LocationEntry>
  display: Map<lowercase class_name -> display>  (back-compat)
                              |
                              v
parseLocationClass(raw, catalog?)
  Tier 0: catalog hit on any token (byName/byTag/bySlug)
          -> hierarchy from entry.star.name + parent.name
  Tier 1: KNOWN_SYSTEMS dict (engine-only short codes)
  Tier 2: KNOWN_BODIES dict (HUR/CRU/ARC/MIC short codes)
  Tier 3: KNOWN_PLACES dict (a few engine-only edge cases)
  Fallback: title-case the raw -> `Other / unmapped`
```

## Changes — Wave 1

| File | Change | Size |
|---|---|---|
| `apps/web/src/lib/reference.ts` | Add `LocationEntry` type and `getLocationCatalog()` returning multi-index map | medium |
| `apps/web/src/lib/class-name-parts.ts` | New `parseLocationClass(raw, catalog)` overload; Tier 0 catalog lookup; shrink dicts | medium |
| `apps/web/src/components/journey/HierarchicalBucketList.tsx` | `rollUpLocations` accepts `LocationCatalog`; threads to parser | small |
| `apps/web/src/app/journey/page.tsx` | `TravelTab` / `CombatTab` call `getLocationCatalog()` instead of `getReferences('location')` | small |
| `docs/REFERENCE-CATALOG-HIERARCHY.md` | This document | — |

**No Rust changes required.** The cron + parse + metadata storage
already work. **No DB migration required.** JSONB is schema-on-read.

## Rollout

1. **Wave 1 (this PR)** — web-side catalog consumption. Locations only.
2. **Wave 2 (later)** — same pattern for weapons + items. Manufacturer
   / family / size all come from `metadata.manufacturer` /
   `metadata.role` / `metadata.hull_size` rather than the TS
   heuristic. Will require auditing the wiki shape for those
   categories — items has 20K+ entries so the entry-map cost matters.

## Fallback semantics

Each tier is independent and additive:

- If catalog fetch fails (404, network, empty `entries`): Tier 0 is
  empty, parser falls through to Tier 1+ dictionaries — same as today.
- If catalog hit but no `star`/`parent` in metadata: parser doesn't
  trust Tier 0 for that token, tries other tiers.
- Dictionaries shrink but don't disappear — engine-only short codes
  (`HUR_L1`, `CRU_L4`) aren't in the wiki and stay hardcoded.

## Synthetic pseudo-systems

The hierarchy `system → body → place` reads as "real astronomy", but
the rollup tree doesn't care about astronomy — it just wants a
three-level grouping. Several engine-internal patterns don't fit
real astronomy at all (they're cross-cutting objects like jump points
or comm arrays), but they slot perfectly into the same tree shape if
we synthesize a top-level "category" that isn't a literal system.

Live pseudo-systems (in `SYNTHETIC_MATCHERS`, applied before the
catalog/dict tiers):

| Pseudo-system | Engine pattern | Example |
|---|---|---|
| `Jump Points` | `rs_ext_<a>-<b>_jp<N>` | `LOC_rs_ext_stan-pyro_jp1` |
| `Communications` | contains `comm_array` / `commarray` | `Comm_Array_Lagrange_Stanton_L1_HUR-L1` |

Engine short-codes routed via `KNOWN_BODIES` (these still attribute
to a real system, but use a *synthetic body* because the engine
doesn't tell us the actual moon/planet):

| Short code | System | Synthetic body | Notes |
|---|---|---|---|
| `RR_*` | Stanton | `Rest Stops` | Lagrange-point rest stations |
| `HDMS_*` | Stanton | `HDMS Outposts` | Hurston Defense Material Storage (real homes: Aberdeen/Magda/Ita/Arial moons) |
| `Shubin_*` | Stanton | `Shubin Outposts` | Shubin Mining facilities (real homes: Calliope/Clio moons) |
| `HUR/CRU/ARC/MIC` | Stanton | Hurston/Crusader/ArcCorp/microTech | Lagrange-point prefixes |
| `HurDyn_*` | Stanton | Hurston | Hurston Dynamics short code |
| `StantonStar/PyroStar/NyxStar` | (their system) | (the system itself) | System-star references |

### Adding a new pseudo-system

Two paths, depending on the engine pattern's shape:

1. **Single short-code prefix** (HDMS, Shubin, etc.) — one entry in
   `KNOWN_BODIES`. The existing Tier 2 body-match handles routing.

2. **Multi-token shape** (Jump Points, Comm Arrays, future patterns
   like Asteroid Belts) — write a `SyntheticMatcher` function and
   add it to `SYNTHETIC_MATCHERS`. Document the engine pattern and
   rationale in this table.

Either way: keep the pseudo-body name human-readable. The user sees
it on the rollup tree.

## Out of scope

- Combining wiki data with our own enrichment (player-supplied location
  names, custom outpost labels). If we ever want that, it goes in a
  separate `location_overrides` table and gets layered on top of the
  catalog.
- Hierarchy for non-location categories — that's Wave 2.
- Localization. Display names are English-only because the wiki API is.
