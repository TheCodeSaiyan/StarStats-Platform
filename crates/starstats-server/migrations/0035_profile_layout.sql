-- 0035_profile_layout.sql
--
-- Owner-side configuration of the public profile widget layout.
-- NULL means "use the default layout encoded in TS" (see
-- apps/web/src/lib/profile-layout.ts DEFAULT_LAYOUT). The web layer is
-- responsible for the projection — the column stores only what the
-- owner has personalised.
--
-- Per docs/ENGINEERING.md migration invariants: additive only, byte-immutable
-- post-deploy. IF NOT EXISTS, no default, no NOT NULL.

ALTER TABLE users
  ADD COLUMN IF NOT EXISTS profile_layout JSONB;
