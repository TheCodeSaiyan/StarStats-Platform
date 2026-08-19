-- 0045_normalize_event_handles.sql
-- Canonicalize events.claimed_handle to lowercase. Handles are
-- case-insensitive identities; a tray re-pair could store events under
-- mixed case, splitting a user's data across the UNIQUE
-- (claimed_handle, idempotency_key) index.

-- Step 1: drop cross-case DUPLICATE events that would collide on the
-- unique index once lowercased (a re-pair re-uploaded the same events
-- under a new case). Keep the lowest-seq row per (lower(handle), idem).
DELETE FROM events a
USING events b
WHERE a.idempotency_key = b.idempotency_key
  AND LOWER(a.claimed_handle) = LOWER(b.claimed_handle)
  AND a.claimed_handle <> b.claimed_handle
  AND a.seq > b.seq;

-- Step 2: lowercase the survivors (no-op for already-lowercase rows).
UPDATE events
SET claimed_handle = LOWER(claimed_handle)
WHERE claimed_handle <> LOWER(claimed_handle);

-- NOTE: audit_log.actor_handle is intentionally NOT normalized here —
-- audit_log is append-only (a trigger rejects UPDATE/DELETE). New rows
-- are lowercased at ingest (PART 1); historical rows stay mixed-case and
-- are served by the case-insensitive read added in 0044 + the
-- LOWER(actor_handle) predicate in ingest_history_for_handle.
