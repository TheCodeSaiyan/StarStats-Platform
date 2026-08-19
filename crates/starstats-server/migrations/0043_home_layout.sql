-- 0043_home_layout.sql
--
-- Second widget-layout surface: the owner's private "Me" home page
-- (/me), independent of the public profile layout in profile_layout
-- (0035). NULL means "use the home default encoded in TS"
-- (apps/web/src/lib/profile-layout.ts). Additive + byte-immutable per
-- docs/ENGINEERING.md migration invariants: IF NOT EXISTS, no default, no NOT NULL.

ALTER TABLE users
  ADD COLUMN IF NOT EXISTS home_layout JSONB;
