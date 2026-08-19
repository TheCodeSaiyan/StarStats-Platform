-- 0034_parser_submissions_audit.sql -- moderator workflow audit column.
--
-- The rule-author moderation UI (W6) lets staff transition each
-- parser_submission row through pending -> drafting -> rule_written
-- (or dismissed), with optional reviewer notes and an outbound
-- `rule_id` once a manifest rule has been authored. Without an
-- `updated_at` timestamp the table records only the *write-side*
-- last_submitted_at (which gets bumped by tray-side resubmissions),
-- which mixes "user submitted again" with "moderator made a decision".
--
-- This migration is additive only — `IF NOT EXISTS` on the column,
-- default `NOW()` so the row populates without backfilling, and a
-- partial index on (status, updated_at) so the admin queue can sort
-- a non-pending bucket by most-recently-touched without a sequential
-- scan when the table grows.
--
-- The column is set automatically by the PATCH handler in
-- `admin_parser_submissions.rs`; the tray-side upsert in
-- `parser_submissions.rs` does NOT touch it, so a fresh tray submit
-- against an existing shape leaves the moderator's last touch intact.

ALTER TABLE parser_submissions
    ADD COLUMN IF NOT EXISTS updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW();

CREATE INDEX IF NOT EXISTS parser_submissions_status_updated
    ON parser_submissions(status, updated_at DESC);
