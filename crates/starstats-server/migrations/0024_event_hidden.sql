-- 0024_event_hidden.sql — Per-event hide-from-shares.
--
-- Adds `hidden_at` to events so the owner can mark individual rows
-- as invisible to shared/public views without deleting them. Default
-- NULL = visible (no behaviour change for existing rows).
--
-- Semantics:
--  * Owner-perspective queries (/v1/me/*) ignore the column — you
--    always see your own events, including hidden ones.
--  * Shared-perspective queries (/v1/u/:handle/*, /v1/public/*)
--    filter `WHERE hidden_at IS NULL`. Hidden events are excluded
--    from totals, by-type breakdown, and the per-day timeline.
--  * Toggling: POST /v1/me/events/:seq/hide sets hidden_at = NOW();
--    DELETE /v1/me/events/:seq/hide nulls it. Both audit-logged.
--
-- Storage: NULL is the common case. A partial index keyed on
-- (claimed_handle, seq DESC) WHERE hidden_at IS NOT NULL stays tiny
-- and speeds up "list my hidden events" without bloating the
-- read-path indexes used by the timeline query.

ALTER TABLE events
    ADD COLUMN IF NOT EXISTS hidden_at TIMESTAMPTZ;

CREATE INDEX IF NOT EXISTS events_handle_hidden_idx
    ON events (claimed_handle, seq DESC)
    WHERE hidden_at IS NOT NULL;
