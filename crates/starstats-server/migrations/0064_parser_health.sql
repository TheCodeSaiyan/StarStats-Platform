-- ============================================================================
-- 0064_parser_health.sql — parser-health detection (additive).
--
-- Detects an event type that has stopped being produced while users remain
-- active. Motivating incident: a Game.log shell tag renamed in the
-- ~2026-07-15 patch and `vehicle_stowed` fell from ~200/day to zero for three
-- weeks with nothing going red.
--
-- Two tables, deliberately:
--   * findings — one row per event_type, with the evidence that produced it
--     and an acknowledge/resolve lifecycle.
--   * runs     — a heartbeat row written on EVERY pass, including clean ones.
--     Without it, "no findings" and "the detector is dead" look identical,
--     which is the exact failure mode this feature exists to catch (the
--     retention sweep only audits when deleted > 0 and has this blind spot).
-- ============================================================================

CREATE TABLE IF NOT EXISTS parser_health_finding (
    event_type       TEXT PRIMARY KEY,
    -- 'dark' (zero events in the recent window) | 'degraded' (reduced share).
    severity         TEXT             NOT NULL,
    -- 'open' | 'acknowledged' | 'resolved'.
    status           TEXT             NOT NULL DEFAULT 'open',
    first_flagged_at TIMESTAMPTZ      NOT NULL DEFAULT now(),
    last_seen_at     TIMESTAMPTZ      NOT NULL DEFAULT now(),

    -- Evidence snapshot, refreshed each pass. Persisted rather than recomputed
    -- so the admin surface can show how much weight the finding carries
    -- (three users going dark at once is stronger than one).
    baseline_events  BIGINT           NOT NULL DEFAULT 0,
    recent_events    BIGINT           NOT NULL DEFAULT 0,
    share_baseline   DOUBLE PRECISION NOT NULL DEFAULT 0,
    share_recent     DOUBLE PRECISION NOT NULL DEFAULT 0,
    baseline_handles BIGINT           NOT NULL DEFAULT 0,
    carried_handles  BIGINT           NOT NULL DEFAULT 0,
    affected_handles BIGINT           NOT NULL DEFAULT 0,

    acknowledged_by  TEXT,
    acknowledged_at  TIMESTAMPTZ,
    note             TEXT,
    resolved_at      TIMESTAMPTZ,
    -- 'recovered' when the type's share came back on its own after a fix
    -- shipped; 'manual' when a human closed it.
    resolved_reason  TEXT
);

CREATE INDEX IF NOT EXISTS parser_health_finding_status_idx
    ON parser_health_finding (status, last_seen_at DESC);

CREATE TABLE IF NOT EXISTS parser_health_run (
    id             BIGSERIAL PRIMARY KEY,
    started_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    finished_at    TIMESTAMPTZ,
    window_start   TIMESTAMPTZ,
    window_end     TIMESTAMPTZ,
    types_examined BIGINT      NOT NULL DEFAULT 0,
    findings_open  BIGINT      NOT NULL DEFAULT 0,
    error          TEXT
);

CREATE INDEX IF NOT EXISTS parser_health_run_started_idx
    ON parser_health_run (started_at DESC);
