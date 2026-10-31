-- RSI handle renames: a signed-in user may change users.claimed_handle,
-- for a typo or because their RSI handle really changed.
--
-- A rename rewrites the handle in place in every handle-keyed table, in
-- one transaction (handle_renames::PostgresHandleRenameStore). This table
-- is the record of each rename, and it does three jobs:
--
--  * Reservation and redirect. The old handle stays held for the user
--    until reserved_until (90 days), so nobody else can claim a
--    just-freed name and impersonate them, and links built on the old
--    handle can be sent on to the new one.
--  * Cooldown. One rename per 30 days, read from users.last_handle_change_at.
--  * SpiceDB reconciler. Postgres and SpiceDB cannot commit together, so
--    the relationship move happens after the commit; a row whose
--    spicedb_moved_at is still NULL is retried by a background pass.
--
-- One-way: additive only (one nullable column, one new table and its
-- indexes). An older server never reads either, so rolling back the
-- binary needs no schema change.

ALTER TABLE users
    ADD COLUMN IF NOT EXISTS last_handle_change_at TIMESTAMPTZ NULL;

CREATE TABLE IF NOT EXISTS handle_renames (
    id                UUID        PRIMARY KEY,
    user_id           UUID        NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    old_handle        TEXT        NOT NULL,
    new_handle        TEXT        NOT NULL,
    -- Whether the account was RSI-verified at the moment of the rename.
    -- The rename resets verification; this keeps what was lost.
    was_verified      BOOLEAN     NOT NULL DEFAULT FALSE,
    -- The one cooldown-free change a never-verified account gets.
    free_change       BOOLEAN     NOT NULL DEFAULT FALSE,
    renamed_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    reserved_until    TIMESTAMPTZ NOT NULL,
    spicedb_moved_at  TIMESTAMPTZ NULL
);

-- Reservation and redirect lookups go by the old handle, case-insensitively,
-- newest reservation first.
CREATE INDEX IF NOT EXISTS handle_renames_old_lc_idx
    ON handle_renames (lower(old_handle), reserved_until DESC);

-- A user's rename history, for the settings pane and the cooldown display.
CREATE INDEX IF NOT EXISTS handle_renames_user_idx
    ON handle_renames (user_id, renamed_at DESC);

-- The reconciler's queue: renames whose SpiceDB move has not landed.
CREATE INDEX IF NOT EXISTS handle_renames_spicedb_pending_idx
    ON handle_renames (renamed_at)
    WHERE spicedb_moved_at IS NULL;
