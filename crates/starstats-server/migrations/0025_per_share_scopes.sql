-- 0025_per_share_scopes.sql — Per-share scope clamps.
--
-- Audit v2 §05.1+§05.5 calls for finer-grained per-share gating than
-- the per-user `users.share_scopes` from 0021: a single owner often
-- wants to give Friend A only the travel timeline while Friend B sees
-- the full manifest. We store the clamp on the (owner, recipient)
-- metadata row so each grant carries its own shape.
--
-- Shape (validated in the handler, not the DB — JSONB is the right
-- store for "small frequently-evolving config" because adding a new
-- field doesn't require a migration):
--   { "kind": "full"|"timeline"|"aggregates"|"tabs",
--     "tabs": [..], "window_days": int|null,
--     "allow_event_types": [..]|null, "deny_event_types": [..]|null }
--
-- NULL = "full manifest" — preserves the legacy behaviour of every
-- already-granted share, so this migration is a no-op at read time
-- until a client explicitly POSTs a scope on grant.

ALTER TABLE share_metadata
    ADD COLUMN IF NOT EXISTS scope JSONB NULL;
