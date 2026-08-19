-- 0022_reference_registry.sql — Generic class-name → display-name registry.
--
-- The previous schema (0012_reference_data.sql) introduced a typed
-- `vehicle_reference` table for ship/vehicle prettification. The
-- catalogue is now expanding to weapons, items, attachments, and
-- locations, so a per-category table per entity type would be a lot
-- of repeated plumbing (migration, sqlx struct, store method, route).
--
-- This migration adds one generic `reference_registry` table keyed on
-- (category, class_name). Per-category metadata lives in a JSONB
-- column — schema-on-read — so adding a new category (e.g. npc)
-- only needs an extension to the CHECK constraint allow-list.
-- Backfilling vehicle data here collapses the legacy typed columns
-- (manufacturer, role, hull_size, focus) into the metadata blob, so
-- the existing /v1/reference/vehicles endpoint can serve the same
-- shape via the new table once the store refactor lands.
--
-- The original `vehicle_reference` table is intentionally left in
-- place. The store-layer refactor (next wave) will swing reads over
-- to the new table; a follow-up migration can drop the legacy one
-- after we've verified the dashboard still resolves vehicle class
-- names cleanly in production.

CREATE TABLE IF NOT EXISTS reference_registry (
    category     TEXT        NOT NULL,
    class_name   TEXT        NOT NULL,
    display_name TEXT        NOT NULL,
    metadata     JSONB       NOT NULL DEFAULT '{}'::jsonb,
    source       TEXT        NOT NULL DEFAULT 'wiki_api',
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (category, class_name)
);

CREATE INDEX IF NOT EXISTS reference_registry_cat_class_lower_idx
    ON reference_registry (category, lower(class_name));

DO $$ BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'reference_registry_category_chk'
    ) THEN
        ALTER TABLE reference_registry
            ADD CONSTRAINT reference_registry_category_chk
            CHECK (category IN ('vehicle', 'weapon', 'item', 'location'));
    END IF;
END $$;

INSERT INTO reference_registry (category, class_name, display_name, metadata, source, updated_at)
SELECT
    'vehicle',
    class_name,
    display_name,
    jsonb_strip_nulls(jsonb_build_object(
        'manufacturer', manufacturer,
        'role',         role,
        'hull_size',    hull_size,
        'focus',        focus
    )),
    'wiki_api',
    updated_at
FROM vehicle_reference
ON CONFLICT (category, class_name) DO NOTHING;
