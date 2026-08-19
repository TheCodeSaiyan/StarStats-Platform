-- 0058_flag_all_handles_dirty.sql
-- One-time backfill for the session/records/entity rollups (0056).
-- Those three tables (session_summary, character_records, entity_rollup_agg)
-- ship empty and are materialized lazily by rebuild_handle_session_stats()
-- on the first read that finds the handle dirty. Historical handles have no
-- stat_rollup_state row yet (only NEW batches create one via insert_batch),
-- so without this backfill their first read would find sessions_dirty absent,
-- read an EMPTY rollup, and undercount. Flag every existing handle dirty so
-- the first post-deploy read recomputes from events. Runs at boot before the
-- server accepts ingest. Idempotent (ON CONFLICT).
INSERT INTO stat_rollup_state (claimed_handle, sessions_dirty, counts_last_seq)
SELECT DISTINCT claimed_handle, TRUE, 0
FROM events
WHERE claimed_handle IS NOT NULL
ON CONFLICT (claimed_handle) DO UPDATE
    SET sessions_dirty = TRUE,
        updated_at = now();
