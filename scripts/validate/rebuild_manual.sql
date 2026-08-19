-- Manual, psql-only trigger of the session/records rollup materialization for
-- one :handle, mirroring PostgresStore::rebuild_handle_session_stats' shipped
-- SQL (repo.rs) WITHOUT the advisory lock / dirty-flag bookkeeping — those are
-- for concurrency, not needed for a single-threaded validation run. Use this to
-- populate the rollups against a real DB, then run 0058_rollup_parity.sql to
-- confirm rollup == live. Idempotent (DELETE + re-INSERT).
--
-- Usage:
--   docker exec -i ss-pg psql -U starstats_app -d starstats \
--     -v handle=thecodesaiyan -f scripts/validate/rebuild_manual.sql
\set gap 30
BEGIN;

-- (1) session_summary
DELETE FROM session_summary WHERE claimed_handle = lower(:'handle');
WITH gaps AS (
    SELECT event_timestamp, event_type,
           LAG(event_timestamp) OVER (ORDER BY event_timestamp ASC) AS prev_ts
    FROM events
    WHERE claimed_handle = lower(:'handle') AND event_timestamp IS NOT NULL
      AND event_type NOT IN ('launcher_activity','game_crash')
), labeled AS (
    SELECT event_timestamp, event_type,
           SUM(CASE WHEN prev_ts IS NULL
                     OR event_timestamp - prev_ts > make_interval(mins => :gap)
                    THEN 1 ELSE 0 END) OVER (ORDER BY event_timestamp ASC) AS session_id
    FROM gaps
)
INSERT INTO session_summary
    (claimed_handle, session_id, started_at, ended_at, event_count, death_count)
SELECT lower(:'handle'), session_id::text,
       MIN(event_timestamp), MAX(event_timestamp),
       COUNT(*)::bigint,
       COUNT(*) FILTER (WHERE event_type = 'player_death')::bigint
FROM labeled GROUP BY session_id;

-- (2) character_records (derived from the freshly-written session_summary + a death-gap scan)
WITH sess AS (
    SELECT started_at, ended_at, event_count, death_count
    FROM session_summary WHERE claimed_handle = lower(:'handle')
), streak AS (
    SELECT MAX(EXTRACT(EPOCH FROM gap))::bigint AS longest_gap
    FROM (SELECT event_timestamp - LAG(event_timestamp) OVER (ORDER BY event_timestamp ASC) AS gap
          FROM events
          WHERE claimed_handle = lower(:'handle')
            AND event_type = 'player_death' AND event_timestamp IS NOT NULL) g
)
INSERT INTO character_records
    (claimed_handle, total_deaths, total_sessions, longest_session_secs,
     busiest_session_events, deadliest_session_deaths,
     longest_survival_gap_secs, first_event_at, last_event_at, updated_at)
SELECT lower(:'handle'),
       (SELECT COALESCE(SUM(death_count),0) FROM sess),
       (SELECT COUNT(*) FROM sess),
       (SELECT COALESCE(MAX(EXTRACT(EPOCH FROM (ended_at - started_at))::bigint),0) FROM sess),
       (SELECT COALESCE(MAX(event_count),0) FROM sess),
       (SELECT COALESCE(MAX(death_count),0) FROM sess),
       (SELECT COALESCE(longest_gap,0) FROM streak),
       (SELECT MIN(started_at) FROM sess),
       (SELECT MAX(ended_at) FROM sess),
       now()
ON CONFLICT (claimed_handle) DO UPDATE SET
    total_deaths = EXCLUDED.total_deaths,
    total_sessions = EXCLUDED.total_sessions,
    longest_session_secs = EXCLUDED.longest_session_secs,
    busiest_session_events = EXCLUDED.busiest_session_events,
    deadliest_session_deaths = EXCLUDED.deadliest_session_deaths,
    longest_survival_gap_secs = EXCLUDED.longest_survival_gap_secs,
    first_event_at = EXCLUDED.first_event_at,
    last_event_at = EXCLUDED.last_event_at,
    updated_at = now();

COMMIT;
