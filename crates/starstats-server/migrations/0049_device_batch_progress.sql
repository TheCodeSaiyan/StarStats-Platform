-- 0049_device_batch_progress.sql -- Per-device ingest-batch high-water mark.
--
-- Online support for the F7 `batch_sequence` gap detector. Each paired
-- device stamps a monotonic per-upload ordinal on its IngestBatch
-- (crates/starstats-core/src/wire.rs). This table remembers the highest
-- ordinal the server has accepted from each device so the /v1/ingest
-- handler can diff the incoming ordinal against it and surface a GAP
-- (a forward jump — uploads lost/dropped) or a REGRESSION (a lower or
-- repeated ordinal — out-of-order arrival, a retry, or a client whose
-- counter reset) as a metric + log line.
--
-- Observability ONLY: a write here never fails or gates ingest, and the
-- handler treats an error as best-effort. Additive, and deliberately no
-- FK to `devices` — a device row can be revoked/deleted without cascading
-- into this diagnostic table, and the key is the token's device_id claim
-- (a string) rather than the devices PK. User-scoped tokens (no device_id)
-- are simply never recorded here.
CREATE TABLE IF NOT EXISTS device_batch_progress (
    device_id           TEXT        PRIMARY KEY,
    last_batch_sequence BIGINT      NOT NULL,
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
