-- 0039_location_taxonomy_v2.sql — Enrichment columns for the location
-- catalogue, derived from a second wiki source (`starcitizen.tools`)
-- and joined onto existing api.star-citizen.wiki rows by slug.
--
-- See docs/PLAN-LOCATION-TAXONOMY-V2.md for the source-quality
-- verdict and the surrounding cross-stack rollout plan, and
-- memory/sc-wiki-location-taxonomy.md for the seven-tier taxonomy
-- this enriches with.
--
-- Adds three nullable columns:
--
--   * `tier`        — coarse top-tier classification (e.g. `landmark`,
--                     `space_station`, `naval_base`). Filterable hot
--                     path on the journey page; gets its own index.
--                     CHECK-constrained allow-list mirrors the
--                     `LocationTier` enum in
--                     `crates/starstats-core/src/location_taxonomy.rs`
--                     (Phase 2). Snake-case to round-trip cleanly
--                     through serde `rename_all = "snake_case"`.
--   * `subtype`     — narrower subtype string (e.g. `drug_lab`,
--                     `rest_stop`, `sealed_settlement`). Open-ended
--                     allow-list — many sub-buckets exist under the
--                     `Landmarks` tier alone — so no CHECK constraint.
--                     Filterable; indexed alongside `category`.
--   * `taxonomy_v2` — JSONB blob holding display-only fields the
--                     filter UI never groups by: `placement`
--                     (`{ kind: 'on_body', body: 'Daymar' }`),
--                     `operator`, `faction`, and a residual
--                     `additional_categories` list for forensics.
--                     Schema-on-read; serde shape lives in Rust.
--
-- The enrichment cron only updates rows that already exist
-- (`category='location'` from the primary api.star-citizen.wiki sync);
-- it never inserts. A starcitizen.tools page with no matching primary
-- entry is skipped (logged for human review). That keeps the engine
-- join key (`tag.name`) authoritative — we'd rather miss enrichment
-- on a location than synthesise a row with no engine binding.
--
-- Per the project migration rule: ADDITIVE ONLY. No DROPs, no NOT
-- NULL on a populated column without a DEFAULT, and the file is
-- byte-immutable post-deploy.

ALTER TABLE reference_registry
    ADD COLUMN IF NOT EXISTS tier TEXT NULL;

ALTER TABLE reference_registry
    ADD COLUMN IF NOT EXISTS subtype TEXT NULL;

ALTER TABLE reference_registry
    ADD COLUMN IF NOT EXISTS taxonomy_v2 JSONB NULL;

-- Tier allow-list. Adding a new tier requires updating both this
-- constraint AND the Rust `LocationTier` enum in lockstep; the
-- integration test `location_taxonomy_round_trips_through_store`
-- (Phase 1 tests) catches drift between the two sides.
DO $$ BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'reference_registry_tier_chk'
    ) THEN
        ALTER TABLE reference_registry
            ADD CONSTRAINT reference_registry_tier_chk
            CHECK (
                tier IS NULL OR tier IN (
                    'system',
                    'astronomical_object',
                    'landing_zone',
                    'space_station',
                    'landmark',
                    'flotilla',
                    'naval_base',
                    'anonymous_poi'
                )
            );
    END IF;
END $$;

-- Guard rail: tier may only be set on location rows. Catches an
-- accidental write from a future enrichment that targets the wrong
-- category. Subtype + taxonomy_v2 ride on the same category check
-- so a single constraint suffices — they're co-populated by the
-- same upsert and never appear independently.
DO $$ BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'reference_registry_taxonomy_category_chk'
    ) THEN
        ALTER TABLE reference_registry
            ADD CONSTRAINT reference_registry_taxonomy_category_chk
            CHECK (
                (tier IS NULL AND subtype IS NULL AND taxonomy_v2 IS NULL)
                OR category = 'location'
            );
    END IF;
END $$;

-- Indexes for the journey-page filter hot path. Partial because
-- the vast majority of `reference_registry` rows are non-location
-- (vehicles + weapons + items dominate by ~100x) and have NULL tier;
-- a non-partial index would bloat with no benefit.
CREATE INDEX IF NOT EXISTS reference_registry_cat_tier_idx
    ON reference_registry (category, tier)
 WHERE tier IS NOT NULL;

CREATE INDEX IF NOT EXISTS reference_registry_cat_subtype_idx
    ON reference_registry (category, subtype)
 WHERE subtype IS NOT NULL;
