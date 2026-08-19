-- 0001_events.sql -- Initial events table.
--
-- One row per ingested log line that the client believed was worth
-- forwarding. Dedupe is on (claimed_handle, idempotency_key).
--
-- Notes:
--  * `id` is uuid v7 generated server-side. Time-ordered → friendly
--    to btree indexes on insertion.
--  * `payload` is the full GameEvent JSON. We keep `raw_line` so we
--    can re-classify with newer parser rules without asking the
--    client to re-upload.
--  * `event_timestamp` is nullable: lines that parse structurally but
--    have no timestamp are still useful and shouldn't be dropped.
--  * `user_id` is intentionally absent for now. It lands in a later
--    migration when auth + the users table arrive; client identity
--    today is the bare `claimed_handle` string.

CREATE TABLE IF NOT EXISTS events (
    id              UUID        PRIMARY KEY,
    idempotency_key TEXT        NOT NULL,
    claimed_handle  TEXT        NOT NULL,
    event_type      TEXT        NOT NULL,
    event_timestamp TIMESTAMPTZ,
    log_source      TEXT        NOT NULL,
    source_offset   BIGINT      NOT NULL,
    raw_line        TEXT        NOT NULL,
    payload         JSONB       NOT NULL,
    received_at     TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE UNIQUE INDEX IF NOT EXISTS events_handle_idem_uq
    ON events (claimed_handle, idempotency_key);

CREATE INDEX IF NOT EXISTS events_handle_received_idx
    ON events (claimed_handle, received_at DESC);

CREATE INDEX IF NOT EXISTS events_type_idx
    ON events (event_type);

CREATE INDEX IF NOT EXISTS events_event_ts_idx
    ON events (event_timestamp DESC) WHERE event_timestamp IS NOT NULL;
