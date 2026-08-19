-- Share-reports moderation queue (audit v2 §05 "Reports queue").
--
-- A reporter (the recipient, or the owner reporting their own grant
-- being misused on the other end) files a report against a specific
-- (owner_handle, recipient_handle) share. The row stays `open` until a
-- moderator resolves it via /v1/admin/sharing/reports/{id}/resolve,
-- which stamps `resolved_at`, `resolved_by`, `resolution_note`.
--
-- Status vocabulary is closed and lives at the application layer:
--   'open'             -- awaiting triage
--   'dismissed'        -- moderator reviewed, no action needed
--   'share_revoked'    -- moderator revoked the underlying share
--   'user_suspended'   -- moderator suspended the owner account
--
-- Reason vocabulary (also closed at the app layer):
--   'abuse', 'spam', 'data_misuse', 'other'
--
-- Idempotent (`IF NOT EXISTS`) so re-applying the migration on an
-- already-migrated DB is a no-op. Same posture as the rest of this
-- migration tree.

CREATE TABLE IF NOT EXISTS share_reports (
    id               UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    reporter_handle  TEXT NOT NULL,
    owner_handle     TEXT NOT NULL,
    recipient_handle TEXT NOT NULL,
    reason           TEXT NOT NULL,
    details          TEXT NULL,
    status           TEXT NOT NULL DEFAULT 'open',
    created_at       TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    resolved_at      TIMESTAMPTZ NULL,
    resolved_by      TEXT NULL,
    resolution_note  TEXT NULL
);

-- Queue index: drives the moderator landing page (default filter:
-- status='open', most-recent-first).
CREATE INDEX IF NOT EXISTS share_reports_status_idx
    ON share_reports (status, created_at DESC);

-- "All reports against this owner" lookups for user-detail admin pages
-- (future wave). Lower-cased to match the case-insensitive handle
-- convention the rest of the sharing surface uses.
CREATE INDEX IF NOT EXISTS share_reports_owner_idx
    ON share_reports (lower(owner_handle), created_at DESC);

-- Rate-limit support: count reports filed by a single reporter in the
-- recent past. Same lower-cased pattern.
CREATE INDEX IF NOT EXISTS share_reports_reporter_idx
    ON share_reports (lower(reporter_handle), created_at DESC);
