-- ============================================================================
-- 0069_events_dedupe.sql — one log line is one event row.
--
-- WHAT IS WRONG. The tray's `idempotency_key` is
-- UUIDv5(log_source : file_sig : offset : line): it identifies a POSITION IN A
-- FILE, not a happening in the game. Game.log is tailed live, rotates into
-- `logbackups/`, and the backfill reads the same bytes again under a different
-- source with a different signature — so the key changes, the unique index on
-- (claimed_handle, idempotency_key) does not fire, and the same event is
-- stored again.
--
-- Measured on one user's tray database, 320,945 rows, every one already
-- uploaded to this server:
--
--     73,052 physical lines stored under more than one log_source
--     153,039 excess rows                                      47.7%
--     deaths 476 stored / 211 real, quantum jumps 2.45x,
--     ship stows 2.31x, planet loads 2.21x
--
-- So every COUNT this server reports is roughly 2.3x too high. Distinct counts
-- are unaffected, which is why "planets visited" looked right while "deaths"
-- did not — and it is very likely why the aggregate reads that prompted the
-- session-rollup work in 0067 were scanning twice the rows they needed to.
--
-- THIS MIGRATION DELETES ROWS. That is stated plainly because the migration
-- rules say a data fix must be: it is not a schema change that happens to
-- touch data. Nothing is lost that cannot be rebuilt — every deleted row is a
-- byte-identical copy of one that remains, and the source log files are on the
-- users' own machines regardless.
--
-- WHY THE SERVER AND NOT JUST THE TRAY. The tray fix ships in the same sweep,
-- but every tray already installed keeps uploading duplicates until its user
-- updates, and there is no way to make them. The guard has to live where the
-- rows land.
--
-- WHY AN EXPRESSION INDEX AND NO NEW COLUMN. A stored hash would have to be
-- computed identically in Rust and in SQL, and `payload::text` is Postgres's
-- normalisation of JSONB (sorted keys, no whitespace) which serde_json does
-- not reproduce byte for byte. Letting Postgres compute the whole thing
-- removes that agreement problem entirely, and an expression index needs no
-- ALTER and no table rewrite.
--
-- `md5` of the two large columns rather than the columns themselves keeps the
-- index key small; an accidental md5 collision across a handle's own rows is
-- not a realistic failure mode here, and the cost of one would be a single
-- dropped duplicate-looking event.
--
-- COALESCE on the timestamp is load-bearing: `event_timestamp` is nullable and
-- NULLs are DISTINCT in a unique index, so without it every timestampless row
-- would be free to duplicate forever.
-- ============================================================================

-- 1. Collapse what is already here.
--
-- Keeps the EARLIEST received copy of each event. `received_at` then `id`
-- gives a total order, so the choice is deterministic rather than whichever
-- row the planner happened to reach first.
WITH ranked AS (
    SELECT id,
           ROW_NUMBER() OVER (
               PARTITION BY claimed_handle,
                            event_type,
                            COALESCE(event_timestamp, '-infinity'::timestamptz),
                            md5(raw_line),
                            md5(payload::text)
               ORDER BY received_at, id
           ) AS rn
      FROM events
)
DELETE FROM events e
 USING ranked r
 WHERE e.id = r.id
   AND r.rn > 1;

-- 2. Stop it happening again.
--
-- The ingest statement uses a bare `ON CONFLICT DO NOTHING`, which covers
-- EVERY unique index on the table rather than one named target — so this index
-- and the older (claimed_handle, idempotency_key) one both silently skip a
-- duplicate instead of failing the batch.
CREATE UNIQUE INDEX IF NOT EXISTS events_content_uq
    ON events (
        claimed_handle,
        event_type,
        COALESCE(event_timestamp, '-infinity'::timestamptz),
        md5(raw_line),
        md5(payload::text)
    );

-- 3. Rebuild the one rollup that cannot heal itself.
--
-- `stat_event_counts` is ACCUMULATED — `insert_batch` adds each batch's counts
-- to the running total rather than recomputing — so deleting the duplicate
-- events above does not touch the totals they already contributed. Every other
-- rollup (`session_summary`, `character_records`, `entity_rollup_agg`) is
-- deleted and rebuilt wholesale for a dirty handle, so step 4 is enough
-- for those.
DELETE FROM stat_event_counts;
INSERT INTO stat_event_counts
    (claimed_handle, event_type, event_count, first_seen_at, last_seen_at)
SELECT lower(claimed_handle),
       event_type,
       COUNT(*),
       MIN(event_timestamp),
       MAX(event_timestamp)
  FROM events
 GROUP BY lower(claimed_handle), event_type
ON CONFLICT (claimed_handle, event_type) DO UPDATE SET
    event_count   = EXCLUDED.event_count,
    first_seen_at = EXCLUDED.first_seen_at,
    last_seen_at  = EXCLUDED.last_seen_at,
    updated_at    = now();

-- 4. Owe every handle a full recompute.
--
-- `sessions_dirty_from_ts = NULL` is not "nothing to do": per 0067 it means
-- the pending range is unknown and a FULL rebuild is still owed, which is
-- exactly right after rows have been removed from underneath the rollups.
-- `counts_last_seq = 0` rewinds the incremental cursor for the same reason.
INSERT INTO stat_rollup_state
    (claimed_handle, sessions_dirty, contracts_dirty, counts_last_seq,
     sessions_dirty_from_ts)
SELECT DISTINCT lower(claimed_handle), TRUE, TRUE, 0, NULL::timestamptz
  FROM events
ON CONFLICT (claimed_handle) DO UPDATE SET
    sessions_dirty         = TRUE,
    contracts_dirty        = TRUE,
    counts_last_seq        = 0,
    sessions_dirty_from_ts = NULL,
    updated_at             = now();
