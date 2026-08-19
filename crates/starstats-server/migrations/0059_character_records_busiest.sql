-- 0059_character_records_busiest.sql
-- character_records (0056) predates the RecordsAggregate shape that
-- records_for_handle returns, which exposes busiest_session_events
-- (MAX events in a single session). Add the column so the records rollup
-- can serve that field. Additive, idempotent, non-superuser safe.
ALTER TABLE character_records
    ADD COLUMN IF NOT EXISTS busiest_session_events BIGINT NOT NULL DEFAULT 0;
