-- 0011_profile_snapshot.sql -- Public RSI citizen-profile snapshots.
--
-- Once a user has proven ownership of their RSI handle (Wave 2,
-- 0009_rsi_verify), Wave 4 fires off a fetch of
-- robertsspaceindustries.com/citizens/{handle} and snapshots the
-- public-facing profile fields: display name, enlistment date,
-- location, badge wall, bio, and primary-org summary.
--
-- One row per snapshot — we keep history (cheap-to-store text +
-- JSONB) so a later wave can render "your profile on date X" and
-- attribute changes (renamed handle, swapped main org). Cleanup is
-- intentionally deferred; the table will grow at most a few rows
-- per user per week and the cost of storing it dwarfs the cost of
-- losing the diff trail.
--
-- PK is `(user_id, captured_at)` because:
--   * snapshots are taken in process time (`NOW()` at insert), so
--     two snapshots for the same user can never share a microsecond
--     timestamp under realistic load;
--   * compound-pk lookups by `user_id` use the index seek directly,
--     which is the dominant access path (`latest_for_user`).
--
-- `latest_for_handle` joins against `users.claimed_handle`. Wave 2's
-- 0004_users.sql already maintains `users_handle_uq` (unique on
-- `lower(claimed_handle)`) so the join is index-driven without
-- adding a fresh index here.
--
-- `badges` is JSONB rather than a side table: badge sets are read
-- as a unit (rendered in the UI), never queried by individual
-- badge, and the row size stays small (max ~20 entries).

CREATE TABLE IF NOT EXISTS rsi_profile_snapshots (
    user_id              UUID        NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    captured_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    display_name         TEXT        NULL,
    enlistment_date      DATE        NULL,
    location             TEXT        NULL,
    bio                  TEXT        NULL,
    primary_org_summary  TEXT        NULL,
    badges               JSONB       NOT NULL DEFAULT '[]'::jsonb,
    PRIMARY KEY (user_id, captured_at)
);

-- Sort the `latest_for_user` lookup off an index without a sort step.
-- The composite PK already supports a forward scan; the descending
-- index gives us O(1) "most recent" via index-only seek.
CREATE INDEX IF NOT EXISTS rsi_profile_snapshots_user_recent_idx
    ON rsi_profile_snapshots (user_id, captured_at DESC);
