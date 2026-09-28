-- Player lookup: find players by the start of their handle, to add them
-- as friends (and, with chat, to message them).
--
--   users.discoverable  NULL (= true) | true | false
--
-- NULL means the default, as users.friend_request_policy does, so existing
-- rows need no backfill. Only players with a verified RSI handle are ever
-- listed, and the lookup needs at least three characters; the column is
-- the player's own opt-out on top of that.
--
-- The index serves `lower(claimed_handle) LIKE 'abc%'`: text_pattern_ops
-- lets a btree answer a prefix match whatever the database collation.
--
-- One-way: additive only (one nullable column and one index); an older
-- server never reads either.

ALTER TABLE users ADD COLUMN IF NOT EXISTS discoverable BOOLEAN NULL;

CREATE INDEX IF NOT EXISTS users_handle_prefix_idx
    ON users (lower(claimed_handle) text_pattern_ops);
