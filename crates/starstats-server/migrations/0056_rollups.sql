-- ============================================================================
-- 0056_rollups.sql — incremental derived-statistics layer (additive).
-- Replaces per-request full-handle scans (summary/records/combat/entities/
-- sessions) with point/range lookups. Maintained in the ingest transaction;
-- reads hard-cut to these tables with a live-computation fallback on miss.
-- Every table is keyed on lower(claimed_handle) (enforced by CHECK).
-- ============================================================================

-- Per-(handle,event_type) running counts. Replaces summary_for_handle scans.
CREATE TABLE IF NOT EXISTS stat_event_counts (
    claimed_handle TEXT        NOT NULL,
    event_type     TEXT        NOT NULL,
    event_count    BIGINT      NOT NULL DEFAULT 0,
    first_seen_at  TIMESTAMPTZ,
    last_seen_at   TIMESTAMPTZ,
    updated_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (claimed_handle, event_type),
    CONSTRAINT stat_event_counts_lc_chk CHECK (claimed_handle = lower(claimed_handle))
);

-- Per-(handle,entity) rollup. Replaces entity GROUP BY + 500k-row Rust walk.
CREATE TABLE IF NOT EXISTS entity_rollup_agg (
    claimed_handle TEXT        NOT NULL,
    entity_kind    TEXT        NOT NULL,
    entity_id      TEXT        NOT NULL,
    display_name   TEXT,
    event_count    BIGINT      NOT NULL DEFAULT 0,
    session_count  BIGINT      NOT NULL DEFAULT 0,
    first_seen_at  TIMESTAMPTZ,
    last_seen_at   TIMESTAMPTZ,
    updated_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (claimed_handle, entity_kind, entity_id),
    CONSTRAINT entity_rollup_agg_lc_chk CHECK (claimed_handle = lower(claimed_handle))
);
CREATE INDEX IF NOT EXISTS entity_rollup_agg_recent_idx
    ON entity_rollup_agg (claimed_handle, last_seen_at DESC NULLS LAST);

-- Per-(handle,session) summary. Serves session list, playtime, session records.
CREATE TABLE IF NOT EXISTS session_summary (
    claimed_handle TEXT        NOT NULL,
    session_id     TEXT        NOT NULL,
    started_at     TIMESTAMPTZ,
    ended_at       TIMESTAMPTZ,
    event_count    BIGINT      NOT NULL DEFAULT 0,
    death_count    BIGINT      NOT NULL DEFAULT 0,
    PRIMARY KEY (claimed_handle, session_id),
    CONSTRAINT session_summary_lc_chk CHECK (claimed_handle = lower(claimed_handle))
);
CREATE INDEX IF NOT EXISTS session_summary_handle_start_idx
    ON session_summary (claimed_handle, started_at DESC NULLS LAST);

-- Per-handle scalar records + combat. Replaces records_for_handle 2 full window
-- scans and the combat 6-scan fan-out.
CREATE TABLE IF NOT EXISTS character_records (
    claimed_handle            TEXT PRIMARY KEY,
    total_deaths              BIGINT      NOT NULL DEFAULT 0,
    total_sessions            BIGINT      NOT NULL DEFAULT 0,
    longest_session_secs      BIGINT,
    deadliest_session_deaths  BIGINT,
    longest_survival_gap_secs BIGINT,
    kills                     BIGINT      NOT NULL DEFAULT 0,
    pvp_deaths                BIGINT      NOT NULL DEFAULT 0,
    first_event_at            TIMESTAMPTZ,
    last_event_at             TIMESTAMPTZ,
    updated_at                TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT character_records_lc_chk CHECK (claimed_handle = lower(claimed_handle))
);

-- Dirty-tracking for the session/records rollups that are not trivially
-- incremental (a late event can merge sessions). Ingest marks a handle dirty;
-- a read-miss or the background refresher recomputes via the SQL sessionizer.
CREATE TABLE IF NOT EXISTS stat_rollup_state (
    claimed_handle  TEXT PRIMARY KEY,
    sessions_dirty  BOOLEAN     NOT NULL DEFAULT TRUE,
    counts_last_seq BIGINT      NOT NULL DEFAULT 0,
    rebuilt_at      TIMESTAMPTZ,
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT stat_rollup_state_lc_chk CHECK (claimed_handle = lower(claimed_handle))
);
CREATE INDEX IF NOT EXISTS stat_rollup_state_dirty_idx
    ON stat_rollup_state (updated_at) WHERE sessions_dirty;
