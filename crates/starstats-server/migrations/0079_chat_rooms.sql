-- Chat rooms (social phase 6): which Matrix rooms the StarStats API made,
-- and whom it put in them.
--
-- Every chat room is created by the API's application service (the chat
-- guard module in infra/synapse refuses anyone else), so this is the full
-- list. It holds no message content: that is end-to-end encrypted in
-- Synapse and never reaches StarStats.
--
--   chat_rooms         one row per room. A crew room belongs to one LFG
--                      post; a DM to one pair of handles, stored lowercased
--                      and ordered so (a, b) and (b, a) are one room.
--   chat_room_members  who the API invited and has not since removed, so
--                      a chat restriction or an account deletion can take
--                      a player out of every room without asking Synapse.
--
-- One-way: additive only (two new tables and their indexes); an older
-- server never reads them.

CREATE TABLE IF NOT EXISTS chat_rooms (
    room_id    TEXT        PRIMARY KEY,
    -- crew | dm
    kind       TEXT        NOT NULL,
    post_id    UUID        NULL,
    dm_a       TEXT        NULL,
    dm_b       TEXT        NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- Set when the room is closed (everyone removed); a closed DM is
    -- replaced by a new room if the pair become friends again.
    closed_at  TIMESTAMPTZ NULL
);

-- One open crew room per post.
CREATE UNIQUE INDEX IF NOT EXISTS chat_rooms_post_uq
    ON chat_rooms (post_id) WHERE kind = 'crew' AND closed_at IS NULL;

-- One open DM per pair.
CREATE UNIQUE INDEX IF NOT EXISTS chat_rooms_dm_uq
    ON chat_rooms (dm_a, dm_b) WHERE kind = 'dm' AND closed_at IS NULL;

CREATE TABLE IF NOT EXISTS chat_room_members (
    room_id    TEXT        NOT NULL REFERENCES chat_rooms(room_id) ON DELETE CASCADE,
    -- lowercased handle
    handle     TEXT        NOT NULL,
    invited_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (room_id, handle)
);

-- "Every room this player is in", for restriction and deletion.
CREATE INDEX IF NOT EXISTS chat_room_members_handle_idx
    ON chat_room_members (handle);
