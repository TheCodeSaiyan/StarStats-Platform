-- 0012_reference_data.sql -- Star Citizen vehicle reference cache.
--
-- The dashboard renders game events whose payloads carry internal
-- class names like `AEGS_Avenger_Stalker_Living`. Players don't
-- recognise those — they expect "Aegis Avenger Stalker". Wave 7
-- introduces a server-side cache of vehicle metadata pulled daily
-- from `https://api.star-citizen.wiki`, keyed on the same internal
-- class name so the rendering layer can join without needing to know
-- about the upstream API.
--
-- One row per vehicle. `class_name` is the natural primary key:
-- the upstream guarantees uniqueness and the events join is on
-- exactly that field. We do NOT use a synthetic UUID because:
--   * the daily refresh is idempotent on (class_name) — surrogate
--     keys would force us to track upstream-id → row-id mapping;
--   * the read path is "lookup by class_name" 100% of the time.
--
-- All metadata fields except `display_name` are nullable: the Wiki
-- API returns inconsistent shapes per vehicle (some have no role,
-- some have no manufacturer record, etc.) and we'd rather store
-- NULL than guess.
--
-- `updated_at` drives a future stale-data ops query — "alert if any
-- row hasn't refreshed in N days, the daily job is broken." Not
-- surfaced via the public response.
--
-- Indexes:
--   * `lower(class_name)` — game logs occasionally vary case on the
--     same class (e.g. `AEGS_Avenger_Stalker` vs `aegs_avenger_stalker`),
--     so the runtime lookup is case-insensitive.

CREATE TABLE IF NOT EXISTS vehicle_reference (
    class_name    TEXT        PRIMARY KEY,
    display_name  TEXT        NOT NULL,
    manufacturer  TEXT        NULL,
    role          TEXT        NULL,
    hull_size     TEXT        NULL,
    focus         TEXT        NULL,
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS vehicle_reference_class_lower_idx
    ON vehicle_reference (lower(class_name));
