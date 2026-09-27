-- Social phase 5: crew history and crew commends.
--
-- Crew history is the private list of players you flew with in a Looking
-- for Group post, so you can find and friend them later. One row per
-- direction of each pair, written when the host accepts a player and
-- deleted if the host later removes them. Ended LFG posts are purged after
-- 30 days, but history is kept for 90, so it cannot be read from
-- lfg_members and keeps its own rows. Purged at 90 days by the LFG
-- retention sweep.
--
-- A commend is one closed-vocabulary word from a crewmate (great pilot,
-- good comms, reliable, good teacher), given in the 48 hours after the
-- post ends. The public totals leave out who gave them. Not a foreign key
-- to lfg_posts: the post is purged at 30 days and the totals must outlive
-- it.
--
-- Handles are keyed case-insensitively, as elsewhere in the social tables.
--
-- One-way: additive only (two new tables and their indexes); an older
-- server never reads them.

CREATE TABLE IF NOT EXISTS crew_history (
    post_id      UUID        NOT NULL,
    handle       TEXT        NOT NULL,
    other_handle TEXT        NOT NULL,
    activity     TEXT        NOT NULL,
    flew_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- One row per direction of a pair per post; accepting twice is a no-op.
CREATE UNIQUE INDEX IF NOT EXISTS crew_history_pair_uq
    ON crew_history (post_id, lower(handle), lower(other_handle));

-- "Players I flew with", newest first.
CREATE INDEX IF NOT EXISTS crew_history_handle_idx
    ON crew_history (lower(handle), flew_at DESC);

-- The 90-day retention sweep.
CREATE INDEX IF NOT EXISTS crew_history_flew_at_idx
    ON crew_history (flew_at);

CREATE TABLE IF NOT EXISTS crew_commends (
    post_id          UUID        NOT NULL,
    giver_handle     TEXT        NOT NULL,
    recipient_handle TEXT        NOT NULL,
    kind             TEXT        NOT NULL,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at       TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- One commend per giver, per recipient, per post.
CREATE UNIQUE INDEX IF NOT EXISTS crew_commends_pair_uq
    ON crew_commends (post_id, lower(giver_handle), lower(recipient_handle));

-- The public totals filter on the recipient.
CREATE INDEX IF NOT EXISTS crew_commends_recipient_idx
    ON crew_commends (lower(recipient_handle), created_at);
