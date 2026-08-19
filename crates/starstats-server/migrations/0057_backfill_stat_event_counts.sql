-- ============================================================================
-- 0057_backfill_stat_event_counts.sql
--
-- Make the stat_event_counts rollup COMPLETE for every existing handle.
--
-- 0056 creates the rollup empty and ingest (insert_batch) only ever counts
-- NEWLY inserted events — historical events hit ON CONFLICT DO NOTHING, so a
-- reparse never re-enters them into the rollup. On an upgraded DB the rollup
-- would therefore hold only post-deploy events, and summary_for_handle (which
-- hard-cuts to it) would UNDERCOUNT once a handle received any new batch.
--
-- This one-time GROUP BY over the source of truth makes the rollup exact for
-- every existing handle; ingest keeps it incrementally correct afterwards, and
-- retention rebuilds a handle's rollup on purge. Runs at boot before the server
-- accepts ingest, so there is no concurrent writer to race.
--
-- ON CONFLICT DO UPDATE = EXCLUDED (full recompute, not DO NOTHING) so it is
-- authoritative even if an earlier build already wrote partial rows for some
-- handle — it overwrites the partial count with the true count from events.
-- ============================================================================

INSERT INTO stat_event_counts (claimed_handle, event_type, event_count, first_seen_at, last_seen_at)
SELECT claimed_handle,
       event_type,
       COUNT(*),
       MIN(event_timestamp),
       MAX(event_timestamp)
FROM events
GROUP BY claimed_handle, event_type
ON CONFLICT (claimed_handle, event_type) DO UPDATE SET
    event_count   = EXCLUDED.event_count,
    first_seen_at = EXCLUDED.first_seen_at,
    last_seen_at  = EXCLUDED.last_seen_at,
    updated_at    = now();
