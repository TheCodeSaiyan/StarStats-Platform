-- Functional indexes so every case-insensitive claimed_handle lookup
-- (sessions, event-read layer, widget queries) uses an index rather
-- than a full table scan.  All are LOWER(claimed_handle) prefixed so
-- they cover the LOWER(claimed_handle) = LOWER($N) predicates added
-- across the EventQuery implementation.

-- sessions_for_handle / total_playtime_secs / count_sessions_since
-- ORDER BY event_timestamp ASC inside the LAG window.
CREATE INDEX IF NOT EXISTS events_lower_handle_ts_idx
    ON events (LOWER(claimed_handle), event_timestamp);

-- list_filtered paginates by seq (ORDER BY seq DESC / seq ASC).
CREATE INDEX IF NOT EXISTS events_lower_handle_seq_idx
    ON events (LOWER(claimed_handle), seq);

-- timeline / timeline_shared / timeline_shared_filtered
-- GROUP BY day uses event_timestamp; covered by events_lower_handle_ts_idx.
-- (No additional index needed — same column order.)

-- event_type_breakdown / summary_for_handle GROUP BY event_type.
CREATE INDEX IF NOT EXISTS events_lower_handle_event_type_idx
    ON events (LOWER(claimed_handle), event_type);

-- latest_location / recent_location_events / location_event_stream
-- ORDER BY event_timestamp DESC; covered by events_lower_handle_ts_idx.
-- (No additional index needed — DESC scans the same B-tree in reverse.)

-- set_event_hidden UPDATE ... WHERE LOWER(claimed_handle) = ... AND seq = ...
-- Covered by events_lower_handle_seq_idx above.

-- ingest_history_for_handle (My logs page) queries audit_log:
--   WHERE action = 'ingest.batch_processed'
--     AND LOWER(actor_handle) = LOWER($1)
--   ORDER BY seq DESC
-- audit_log has no LOWER(actor_handle) index, so the case-insensitive
-- predicate would full-scan an append-only (unbounded) table. Cover it.
CREATE INDEX IF NOT EXISTS audit_log_lower_actor_seq_idx
    ON audit_log (LOWER(actor_handle), seq DESC);
