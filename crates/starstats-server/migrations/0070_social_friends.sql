-- Social phase 1: friends, blocks, mutes and in-app notifications.
--
-- Everything is keyed by handle, like the sharing surface
-- (share_metadata, share_reports), and compared case-insensitively
-- through lower(...) indexes for the same reason.
--
-- Vocabularies are closed at the application layer and stored as TEXT,
-- so adding a variant never needs a migration:
--   friend_requests.status  'pending' | 'accepted' | 'declined' | 'cancelled'
--   notifications.kind       'friend_request' | 'friend_accepted'
--   users.friend_request_policy  NULL (= 'everyone') | 'everyone' | 'nobody'
--
-- One-way: additive only. Rolling the application back leaves these
-- tables unread and harmless; nothing existing is altered except one
-- nullable column on users.

CREATE TABLE IF NOT EXISTS friend_requests (
    id               UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    requester_handle TEXT NOT NULL,
    recipient_handle TEXT NOT NULL,
    status           TEXT NOT NULL DEFAULT 'pending',
    created_at       TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    responded_at     TIMESTAMPTZ NULL
);

-- At most one pending request per direction. Enforced here as well as
-- in the handler so a double-click race cannot create two.
CREATE UNIQUE INDEX IF NOT EXISTS friend_requests_one_pending_idx
    ON friend_requests (lower(requester_handle), lower(recipient_handle))
    WHERE status = 'pending';

-- "Incoming requests" list, and the rate-limit count on the sender side.
CREATE INDEX IF NOT EXISTS friend_requests_recipient_idx
    ON friend_requests (lower(recipient_handle), created_at DESC);
CREATE INDEX IF NOT EXISTS friend_requests_requester_idx
    ON friend_requests (lower(requester_handle), created_at DESC);

-- One row per friendship. handle_a sorts before handle_b (lower-cased),
-- so a pair has exactly one representation and the unique index holds.
CREATE TABLE IF NOT EXISTS friendships (
    handle_a   TEXT NOT NULL,
    handle_b   TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE UNIQUE INDEX IF NOT EXISTS friendships_pair_idx
    ON friendships (lower(handle_a), lower(handle_b));
-- The pair index serves lookups by handle_a; this one serves handle_b.
CREATE INDEX IF NOT EXISTS friendships_b_idx
    ON friendships (lower(handle_b));

CREATE TABLE IF NOT EXISTS user_blocks (
    blocker_handle TEXT NOT NULL,
    blocked_handle TEXT NOT NULL,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE UNIQUE INDEX IF NOT EXISTS user_blocks_pair_idx
    ON user_blocks (lower(blocker_handle), lower(blocked_handle));
-- "Has anyone blocked me?" checks run from the blocked side.
CREATE INDEX IF NOT EXISTS user_blocks_blocked_idx
    ON user_blocks (lower(blocked_handle));

CREATE TABLE IF NOT EXISTS user_mutes (
    muter_handle TEXT NOT NULL,
    muted_handle TEXT NOT NULL,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE UNIQUE INDEX IF NOT EXISTS user_mutes_pair_idx
    ON user_mutes (lower(muter_handle), lower(muted_handle));

CREATE TABLE IF NOT EXISTS notifications (
    id               UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    recipient_handle TEXT NOT NULL,
    kind             TEXT NOT NULL,
    actor_handle     TEXT NULL,
    payload          JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    read_at          TIMESTAMPTZ NULL
);

-- Drives the inbox (newest first, `since` cursor) and the unread count.
CREATE INDEX IF NOT EXISTS notifications_recipient_idx
    ON notifications (lower(recipient_handle), created_at DESC);

ALTER TABLE users ADD COLUMN IF NOT EXISTS friend_request_policy TEXT NULL;
