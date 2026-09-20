-- ============================================================================
-- 0067_session_rollup_watermark.sql — earliest-pending-event watermark for the
-- session rollup, so a rebuild can be INCREMENTAL instead of total.
--
-- `sessions_dirty` is a boolean: it says something changed, never what. The
-- only safe response to "something changed" is to recompute everything, and
-- that is what the rollup did — a parallel seq scan of the whole events table
-- per rebuild (measured: 831 ms and ~2.5 GB of buffer reads for a 440k-event
-- handle), triggered on every read because ingest re-dirties the row faster
-- than a rebuild can clear it.
--
-- WHY A TIMESTAMP AND NOT A SEQUENCE. `seq` is ingest order; `event_timestamp`
-- is game order, and the tray's catch-up drain makes them diverge by hours or
-- days. A late event does not merely append — landing inside an old idle gap
-- it MERGES two previously separate sessions, which 0056's own comment warns
-- about ("not trivially incremental (a late event can merge sessions)"). So
-- "everything after seq N" is the wrong set of rows; the affected range starts
-- at the EARLIEST TIMESTAMP that arrived, wherever in history that falls.
--
-- (`counts_last_seq`, added in 0056, is a seq watermark that was never read or
-- advanced — it is left untouched here rather than repurposed, since this
-- column answers a different question.)
--
-- NULL means "nothing pending": either never dirtied, or fully rebuilt. The
-- existing `sessions_dirty` boolean is deliberately NOT dropped — migrations
-- here are additive only, and it keeps working as the coarse signal while the
-- watermark is populated and proven.
-- ============================================================================

ALTER TABLE stat_rollup_state
    ADD COLUMN IF NOT EXISTS sessions_dirty_from_ts TIMESTAMPTZ;

-- Existing dirty rows have no watermark, and we cannot know what their pending
-- range was. NULL-with-dirty is read as "recompute everything once", which is
-- the pre-migration behaviour — so the first rebuild after deploy is a full
-- one and every rebuild after it is incremental.
COMMENT ON COLUMN stat_rollup_state.sessions_dirty_from_ts IS
    'Earliest event_timestamp ingested since the last successful session rollup rebuild. NULL = nothing pending (or unknown, for rows dirtied before 0067, which force one full rebuild).';
