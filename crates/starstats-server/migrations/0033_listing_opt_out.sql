-- Per-user opt-out from the public-profile listing at /discover.
--
-- Piece 4 of the public-profile UX work. Public visibility itself is
-- a SpiceDB relation (`stats_record:<handle>#public_view@user:*`), but
-- inclusion in the discoverable listing is a separate concern: a user
-- can be willing to be reachable by a direct URL while still wanting
-- to stay off a browsable list. Storing the opt-out as a per-user
-- SQL boolean (rather than as another SpiceDB relation) avoids
-- modelling a second relation just to express "exclude from listing"
-- — there is no relational reason to put this in SpiceDB.
--
-- Default FALSE means "appear in the listing when public" — opt-out,
-- not opt-in, matches the documented Piece 4 product behaviour and
-- keeps existing public profiles surfaced after the migration.
--
-- Migrations are additive only: IF NOT EXISTS on the column and the
-- partial index so a re-run on an environment already on this
-- version is a no-op. The partial index covers only the rows the
-- `/discover` query actually selects (`listing_opt_out = FALSE`),
-- so it stays small even as the users table grows.
ALTER TABLE users
    ADD COLUMN IF NOT EXISTS listing_opt_out BOOLEAN NOT NULL DEFAULT FALSE;

CREATE INDEX IF NOT EXISTS users_listing_opt_out
    ON users(listing_opt_out) WHERE listing_opt_out = FALSE;
