-- 0053_parser_submission_attribution.sql
-- Links tray parser_submissions to the public community submissions
-- queue, and records the tray user's opt-in attribution choice.
-- Additive only: all columns nullable, no DROP, no NOT NULL.

ALTER TABLE parser_submissions
    ADD COLUMN IF NOT EXISTS submitter_user_id       UUID NULL REFERENCES users(id) ON DELETE SET NULL,
    ADD COLUMN IF NOT EXISTS submitter_handle        TEXT NULL,
    ADD COLUMN IF NOT EXISTS community_submission_id UUID NULL REFERENCES submissions(id) ON DELETE SET NULL;

ALTER TABLE submissions
    ADD COLUMN IF NOT EXISTS source_shape_hash TEXT NULL;

-- One community row per tray shape (DB-enforced idempotency).
CREATE UNIQUE INDEX IF NOT EXISTS submissions_source_shape_uq
    ON submissions (source_shape_hash)
    WHERE source_shape_hash IS NOT NULL;
