-- 0047_quarantined_events.sql -- Quarantine for rejected ingest events.
--
-- Events that fail server-side validation at POST /v1/ingest are NOT
-- written to `events` (they would skew every downstream metric).
-- Instead of silently dropping them, we retain them here so a maintainer
-- can see WHICH handle/collector is producing invalid data and WHY --
-- the "diagnosable, not silent" posture the telemetry audit (F5) calls
-- for. Read/analysed out-of-band by maintainers; no read endpoint yet.
--
-- Same trust domain as `events` (the submitter's own data), so the raw
-- line + payload are retained verbatim for diagnosis. Additive, no FK.
--
-- Idempotent on (claimed_handle, idempotency_key): a retried bad batch
-- re-submits the same rejected events, and the unique index + the
-- handler's ON CONFLICT DO NOTHING keep the table from bloating.

CREATE TABLE IF NOT EXISTS quarantined_events (
    id              UUID        PRIMARY KEY,
    idempotency_key TEXT        NOT NULL,
    claimed_handle  TEXT        NOT NULL,
    -- Coarse machine-readable bucket, e.g. 'validation'.
    reason          TEXT        NOT NULL,
    -- Human-readable specifics, e.g. the validation error text.
    detail          TEXT,
    log_source      TEXT        NOT NULL,
    source_offset   BIGINT      NOT NULL,
    raw_line        TEXT        NOT NULL,
    payload         JSONB,
    quarantined_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE UNIQUE INDEX IF NOT EXISTS quarantined_events_handle_idem_uq
    ON quarantined_events (claimed_handle, idempotency_key);

CREATE INDEX IF NOT EXISTS quarantined_events_handle_time_idx
    ON quarantined_events (claimed_handle, quarantined_at DESC);

CREATE INDEX IF NOT EXISTS quarantined_events_reason_idx
    ON quarantined_events (reason);
