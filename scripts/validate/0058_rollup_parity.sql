-- Parity harness for rebuild_handle_session_stats. Run AFTER calling the
-- rebuild for :handle. Each block must return ZERO rows (rollup == live).
\set gap 30
-- 1. session_summary vs a live sessionizer GROUP BY. session_id is an ordinal
--    label cast to text on both sides so the EXCEPT column types match.
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
), live AS (
    SELECT session_id::text AS session_id,
           MIN(event_timestamp) AS started_at, MAX(event_timestamp) AS ended_at,
           COUNT(*)::bigint AS event_count,
           COUNT(*) FILTER (WHERE event_type = 'player_death')::bigint AS death_count
    FROM labeled GROUP BY session_id
)
(SELECT session_id, started_at, ended_at, event_count, death_count FROM live
 EXCEPT
 SELECT session_id, started_at, ended_at, event_count, death_count
 FROM session_summary WHERE claimed_handle = lower(:'handle'))
UNION ALL
(SELECT session_id, started_at, ended_at, event_count, death_count
 FROM session_summary WHERE claimed_handle = lower(:'handle')
 EXCEPT
 SELECT session_id, started_at, ended_at, event_count, death_count FROM live);
-- 2. character_records vs a live gap-sessionized recompute. The rebuild stores
--    COALESCE(...,0) for the record fields, so the live side coalesces too. One
--    row each (the rebuild always inserts a row, all-zero for an empty handle).
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
), sess AS (
    SELECT EXTRACT(EPOCH FROM (MAX(event_timestamp) - MIN(event_timestamp)))::bigint AS dur_secs,
           COUNT(*)::bigint AS ev_count,
           COUNT(*) FILTER (WHERE event_type = 'player_death')::bigint AS death_count
    FROM labeled GROUP BY session_id
), streak AS (
    SELECT MAX(EXTRACT(EPOCH FROM gap))::bigint AS longest_gap
    FROM (SELECT event_timestamp - LAG(event_timestamp) OVER (ORDER BY event_timestamp ASC) AS gap
          FROM events
          WHERE claimed_handle = lower(:'handle')
            AND event_type = 'player_death' AND event_timestamp IS NOT NULL) g
), live_rec AS (
    SELECT COALESCE(SUM(death_count),0)::bigint            AS total_deaths,
           COUNT(*)::bigint                                AS total_sessions,
           COALESCE(MAX(dur_secs),0)::bigint               AS longest_session_secs,
           COALESCE(MAX(ev_count),0)::bigint               AS busiest_session_events,
           COALESCE(MAX(death_count),0)::bigint            AS deadliest_session_deaths,
           (SELECT COALESCE(longest_gap,0) FROM streak)::bigint AS longest_survival_gap_secs
    FROM sess
)
(SELECT total_deaths, total_sessions, longest_session_secs, busiest_session_events,
        deadliest_session_deaths, longest_survival_gap_secs FROM live_rec
 EXCEPT
 SELECT total_deaths, total_sessions, longest_session_secs, busiest_session_events,
        deadliest_session_deaths, longest_survival_gap_secs
 FROM character_records WHERE claimed_handle = lower(:'handle'))
UNION ALL
(SELECT total_deaths, total_sessions, longest_session_secs, busiest_session_events,
        deadliest_session_deaths, longest_survival_gap_secs
 FROM character_records WHERE claimed_handle = lower(:'handle')
 EXCEPT
 SELECT total_deaths, total_sessions, longest_session_secs, busiest_session_events,
        deadliest_session_deaths, longest_survival_gap_secs FROM live_rec);
-- NOTE: entity_rollup_agg is intentionally NOT materialized yet (A4 deferred,
-- blocked on item B's process_init sessionizer), so it has no parity block here.
