-- 0031_share_event_timeline.sql -- Per-share toggle for the new
-- per-event timeline endpoint.
--
-- Phase 6 of the audit-v2 event-handling spec ships the first
-- per-event shared-timeline surface (`/v1/users/{handle}/sessions[/...]`).
-- That surface exposes individual log lines, not the coarse day-bucket
-- counts that `share_with_user` was originally scoped to, so we gate
-- it behind a separate boolean rather than collapsing the affordance
-- into the existing share grant.
--
-- Semantics:
--   * NULL or FALSE = the recipient sees the same data they saw
--     before this column existed (summary + day-bucket timeline). No
--     access to the new per-event endpoints.
--   * TRUE = the recipient additionally sees the per-event timeline
--     and session list for the owner.
--
-- Default FALSE on every existing row preserves the conservative
-- posture: nobody gets a new affordance until the owner ticks the
-- box. Toggling lives on the existing share_metadata row (an upsert
-- on (owner_handle, recipient_handle) flips the bit).
--
-- The column is intentionally NOT NULL with a default rather than
-- nullable + treated-as-false. We don't need to distinguish "set to
-- false" from "never set" — both mean the same thing — and a strict
-- bool keeps query predicates simple (`WHERE share_event_timeline`).

ALTER TABLE share_metadata
    ADD COLUMN IF NOT EXISTS share_event_timeline BOOLEAN NOT NULL DEFAULT FALSE;
