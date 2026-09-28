-- Chat reports (social phase 6): a player reports another player in a chat
-- room they share, revealing the messages they choose.
--
-- Chat is end-to-end encrypted, so StarStats holds no message content.
-- A report is the one exception, by the reporter's choice: the messages
-- they tick are sent here in plain text, and only moderators see them.
-- Matrix has no message franking, so a revealed message cannot prove its
-- sender; moderators are told so, and weigh it.
--
-- Kept after the room goes, so a decision can be reviewed, as LFG and
-- share reports are. Account deletion removes the rows that name the
-- reporter or the reported player (social::delete_social_rows_for).
--
-- One-way: additive only (one new table and its indexes); an older server
-- never reads it.

CREATE TABLE IF NOT EXISTS chat_reports (
    id               UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    reporter_handle  TEXT        NOT NULL,
    reported_handle  TEXT        NOT NULL,
    room_id          TEXT        NOT NULL,
    -- harassment | spam | scam | illegal_content | other
    reason           TEXT        NOT NULL,
    details          TEXT        NULL,
    -- [{event_id, sender, sent_at, text}], as the reporter revealed them.
    messages         JSONB       NOT NULL,
    -- open | dismissed | chat_restricted | user_suspended
    status           TEXT        NOT NULL DEFAULT 'open',
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    resolved_at      TIMESTAMPTZ NULL,
    resolved_by      TEXT        NULL,
    resolution_note  TEXT        NULL
);

-- The moderation queue, newest first within a status.
CREATE INDEX IF NOT EXISTS chat_reports_status_idx
    ON chat_reports (status, created_at DESC);

-- The per-reporter daily limit.
CREATE INDEX IF NOT EXISTS chat_reports_reporter_idx
    ON chat_reports (lower(reporter_handle), created_at DESC);
