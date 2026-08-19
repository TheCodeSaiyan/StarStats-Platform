-- 0030_events_metadata.sql -- EventMetadata JSONB envelope on events.
--
-- Phase 1 (audit-v2 event-handling spec) added a
-- `metadata: Option<EventMetadata>` field to the wire `EventEnvelope`
-- and threaded it through `StoredEvent`, but the column was deferred
-- to this follow-up so the schema bump could land additively.
--
-- Semantics:
--  * NULL = legacy row written before this migration (or a future
--    edge where the server didn't synthesise — see the v1 backfill
--    in `ingest::handle`). Reads that group by metadata simply skip
--    NULL rows; no fan-out, no default coerced in SQL.
--  * The ingest path writes one of:
--    - the producer-supplied metadata (v2 client), or
--    - server-synthesised default Observed metadata (v1 client,
--      back-filled by `metadata::stamp` before insert).
--  * On `INSERT ... ON CONFLICT` the existing dedup behaviour
--    (`claimed_handle, idempotency_key` unique) is preserved, but
--    the metadata column COALESCEs so a later v2 retry of an
--    originally-v1 row can land its richer metadata without
--    clobbering a pre-existing value. See `PostgresStore::insert`.
--
-- Indexes follow the canonical access patterns:
--  * `group_key`            -> entity-grouped timeline collapse.
--  * `(kind, id)` on the    -> per-entity rollups across sessions.
--    primary_entity object
--  * `source = 'inferred'`  -> "show me only the inferred rows"
--    filter (small slice of the table, partial index keeps it cheap).
--
-- All three indexes are JSONB path expressions on the (low-cost)
-- top-level keys; they avoid GIN to stay narrow — the queries only
-- ever exact-match these fields.

ALTER TABLE events
    ADD COLUMN IF NOT EXISTS metadata JSONB;

CREATE INDEX IF NOT EXISTS events_metadata_group_key
    ON events ((metadata->>'group_key'))
    WHERE metadata IS NOT NULL;

CREATE INDEX IF NOT EXISTS events_metadata_entity
    ON events ((metadata->'primary_entity'->>'kind'), (metadata->'primary_entity'->>'id'))
    WHERE metadata IS NOT NULL;

CREATE INDEX IF NOT EXISTS events_metadata_inferred
    ON events ((metadata->>'source'))
    WHERE metadata IS NOT NULL AND metadata->>'source' = 'inferred';
