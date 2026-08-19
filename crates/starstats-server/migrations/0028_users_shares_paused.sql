-- Audit v2.1 §C abuse-signal auto-pause.
--
-- When a user accumulates the cross-report cluster threshold (>= 3
-- reports against their shares inside a 72h window — see
-- `check_cross_report_cluster` in sharing_routes.rs), the report
-- handler stamps `shares_paused_until` on the OWNER's row with a
-- short ban (e.g. 24h). The `add_share` handler reads this column on
-- the first thing it does after auth and rejects with 403 when the
-- value is in the future.
--
-- NULL = never paused (the default for every existing row + every
-- new signup). When the timestamp passes, the gate falls open
-- automatically — no separate "unpause" cron needed for the v1
-- cycle. A moderator can also clear it manually via the admin
-- surface when the by-user sub-tab grows a control for it.
--
-- Idempotent so re-applying the migration on an already-migrated
-- DB is a no-op. Same posture as the rest of the migration tree.

ALTER TABLE users
    ADD COLUMN IF NOT EXISTS shares_paused_until TIMESTAMPTZ NULL;

-- Partial index — the gate only ever asks "is this user currently
-- paused?", so we only need to index rows where the column is set.
-- That keeps the index trivially small (zero rows in the steady
-- state) and dodges write amplification on every signup.
CREATE INDEX IF NOT EXISTS users_shares_paused_until_idx
    ON users (lower(claimed_handle))
    WHERE shares_paused_until IS NOT NULL;
