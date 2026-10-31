-- The player's chat (Matrix) localpart, pinned at their first handle rename.
--
-- A chat user ID is @<lowercased handle>:<server>, and Synapse cannot
-- rename a user ID. So a renamed player keeps the chat account they
-- already have: the rename stores the old localpart here, and the new
-- handle becomes the chat display name. NULL means "derive it from the
-- handle", which is what every never-renamed account does.
--
-- The index stops two accounts pinning the same localpart. It cannot see
-- a derived one: once the old handle's 90-day reservation ends, a new
-- account may claim that handle, and the chat code must then pin a
-- different localpart for it rather than derive the one already in use.
--
-- One-way: additive only (one nullable column and a partial index). An
-- older server never reads it.

ALTER TABLE users
    ADD COLUMN IF NOT EXISTS chat_localpart TEXT NULL;

CREATE UNIQUE INDEX IF NOT EXISTS users_chat_localpart_uq
    ON users (chat_localpart)
    WHERE chat_localpart IS NOT NULL;
