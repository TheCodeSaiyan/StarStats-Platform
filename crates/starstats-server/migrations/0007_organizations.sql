-- 0007_organizations.sql -- Organizations (the "share with org" half).
--
-- Membership is NOT tracked here. The single source of truth for who
-- belongs to which org and at what role lives in SpiceDB:
--   organization:<slug>#owner@user:<handle>
--   organization:<slug>#admin@user:<handle>
--   organization:<slug>#member@user:<handle>
--
-- This table holds metadata only — display name, the URL-safe slug,
-- and the original creator (`owner_user_id`) so we can list "orgs you
-- own" without a SpiceDB ReadRelationships round trip on the
-- /v1/orgs landing endpoint.
--
-- The `slug` column is the application identifier used in SpiceDB
-- object IDs (organization:<slug>). It must stay stable for the
-- lifetime of the org — renaming the display name does NOT change
-- the slug. Generated server-side from `name` (lowercase, ASCII,
-- [a-z0-9-], collapse repeats, max 64 chars). On collision, the
-- handler appends `-2`, `-3`, ... up to `-9`; further collisions
-- return 409.

CREATE TABLE IF NOT EXISTS organizations (
    id            UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    name          TEXT        NOT NULL,
    slug          TEXT        NOT NULL UNIQUE,
    owner_user_id UUID        NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS organizations_owner_idx
    ON organizations (owner_user_id);
