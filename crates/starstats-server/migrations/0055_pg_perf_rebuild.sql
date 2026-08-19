-- ============================================================================
-- 0055_pg_perf_rebuild.sql — performance rebuild, ADDITIVE form.
--
-- Reproduces the events index/constraint deltas of the (reverted) consolidated
-- baseline on top of the existing 54-migration history, so it applies cleanly
-- against a DEPLOYED database without a schema reset. Consolidating the 54
-- migrations into a fresh baseline was fatal against an existing DB (sqlx:
-- "migration 3 was previously applied but is missing"); this restores the
-- additive-only invariant. See docs/audit/postgres-performance-review-2026-07-22.md.
-- ============================================================================
-- NOTE: pg_stat_statements is NOT created here — CREATE EXTENSION requires
-- superuser, which the app role (starstats_app) is not. It is provisioned by
-- the init container (infra/init/init-databases.sh) as the postgres superuser.
-- ============================================================================

-- Drop 3 dead + 2 redundant LOWER() indexes on the hot events table.
--   events_type_idx / events_event_ts_idx : low-selectivity / global-time,
--     no cross-user query justifies them (events is never aggregated cross-user).
--   events_lower_handle_* : redundant once queries filter the bare (lowercase)
--     claimed_handle column instead of LOWER(claimed_handle).
DROP INDEX IF EXISTS events_type_idx;
DROP INDEX IF EXISTS events_event_ts_idx;
DROP INDEX IF EXISTS events_lower_handle_seq_idx;
DROP INDEX IF EXISTS events_lower_handle_ts_idx;
DROP INDEX IF EXISTS events_lower_handle_event_type_idx;

-- Plain-column composites the LOWER()-stripped queries use (index-only-scan
-- capable; the retained events_handle_seq_idx / events_handle_received_idx cover
-- seq + received_at).
CREATE INDEX IF NOT EXISTS events_handle_ts_idx
    ON events (claimed_handle, event_timestamp);
CREATE INDEX IF NOT EXISTS events_handle_event_type_idx
    ON events (claimed_handle, event_type);

-- Formalize the ingest lowercase invariant (ingest.rs lowercases every handle;
-- migration 0045 normalized existing rows). NOT VALID so a deploy never blocks
-- on a legacy stray row — the constraint is enforced for all new writes, which
-- is what the query rewrite relies on. Guarded so re-runs are no-ops.
DO $$ BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint WHERE conname = 'events_handle_lowercase_chk'
    ) THEN
        ALTER TABLE events
            ADD CONSTRAINT events_handle_lowercase_chk
            CHECK (claimed_handle = lower(claimed_handle)) NOT VALID;
    END IF;
END $$;

-- events sees heavy append + periodic retention DELETE churn; vacuum/analyze
-- sooner than the lazy 20%/10% defaults.
ALTER TABLE events SET (
    autovacuum_vacuum_scale_factor  = 0.02,
    autovacuum_analyze_scale_factor = 0.01,
    autovacuum_vacuum_cost_limit    = 2000
);
