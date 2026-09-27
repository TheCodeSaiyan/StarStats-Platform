-- Social phase 4: the Looking for Group board.
--
-- A post is a host's short-lived call for crew: an activity, where, the
-- ship, how many crew, and an expiry. Other players ask to join; the host
-- accepts, declines or removes them. Posts are reportable, and a report
-- keeps a snapshot of the post so a moderator can see what was reported
-- after the post itself has expired or been removed.
--
-- Handles are keyed case-insensitively, as elsewhere in the social tables.
-- Vocabularies (activity, voice, region, member status, report reason and
-- status) are closed in Rust and stored as TEXT, so adding a value needs no
-- migration.
--
-- One-way: additive only (three new tables and their indexes); an older
-- server never reads them.

CREATE TABLE IF NOT EXISTS lfg_posts (
    id             UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    host_handle    TEXT        NOT NULL,
    activity       TEXT        NOT NULL,
    system         TEXT        NULL,
    location       TEXT        NULL,
    ship           TEXT        NULL,
    crew_slots     SMALLINT    NOT NULL,
    voice          TEXT        NOT NULL,
    region         TEXT        NOT NULL,
    note           TEXT        NULL,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at     TIMESTAMPTZ NOT NULL,
    -- Closed by the host. NULL while open.
    closed_at      TIMESTAMPTZ NULL,
    -- Taken down by a moderator. NULL unless removed.
    removed_at     TIMESTAMPTZ NULL,
    removed_by     TEXT        NULL,
    removed_reason TEXT        NULL
);

-- The board: posts still open (the query also drops expired ones).
CREATE INDEX IF NOT EXISTS lfg_posts_open_idx
    ON lfg_posts (expires_at DESC)
    WHERE closed_at IS NULL AND removed_at IS NULL;

-- "One open post per host" and the per-host posting rate limit.
CREATE INDEX IF NOT EXISTS lfg_posts_host_idx
    ON lfg_posts (lower(host_handle), created_at DESC);

CREATE TABLE IF NOT EXISTS lfg_members (
    post_id       UUID        NOT NULL REFERENCES lfg_posts(id) ON DELETE CASCADE,
    member_handle TEXT        NOT NULL,
    -- requested | accepted | declined | left | removed
    status        TEXT        NOT NULL,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    responded_at  TIMESTAMPTZ NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS lfg_members_pair_uq
    ON lfg_members (post_id, lower(member_handle));

CREATE TABLE IF NOT EXISTS lfg_reports (
    id              UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    -- Not a foreign key: the report outlives the post it is about.
    post_id         UUID        NOT NULL,
    reporter_handle TEXT        NOT NULL,
    host_handle     TEXT        NOT NULL,
    reason          TEXT        NOT NULL,
    details         TEXT        NULL,
    -- The post as the reporter saw it, for the moderator.
    post_snapshot   JSONB       NOT NULL,
    status          TEXT        NOT NULL DEFAULT 'open',
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    resolved_at     TIMESTAMPTZ NULL,
    resolved_by     TEXT        NULL,
    resolution_note TEXT        NULL
);

-- The moderation queue, newest first within a status.
CREATE INDEX IF NOT EXISTS lfg_reports_status_idx
    ON lfg_reports (status, created_at DESC);

-- The per-reporter rate limit.
CREATE INDEX IF NOT EXISTS lfg_reports_reporter_idx
    ON lfg_reports (lower(reporter_handle), created_at DESC);
