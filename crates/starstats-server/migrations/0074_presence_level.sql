-- Social phase 3: the server-side gate for friends-only presence.
--
-- How much of a user's presence their friends may see: 'off', 'status'
-- (offline / online / in game / in quantum) or 'system' (status plus the
-- star system). NULL reads as 'off', so nobody shares until they choose
-- to and existing rows need no backfill.
--
-- Presence itself is never stored: it lives in the API's memory and
-- expires minutes after the tray stops reporting. This column is the only
-- presence data in the database.
--
-- One-way: additive only (a nullable column); an older server never
-- reads it.

ALTER TABLE users ADD COLUMN IF NOT EXISTS presence_level TEXT NULL;
