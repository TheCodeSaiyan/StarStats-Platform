-- 0013_hangar.sql -- Hangar snapshot (RSI-owned ships).
--
-- The desktop tray client periodically scrapes the user's RSI account
-- pledge ledger (https://robertsspaceindustries.com/account/pledges)
-- against their RSI session cookie -- the cookie itself never leaves the
-- user's machine. The tray ships a structured `Vec<HangarShip>` plus a
-- captured-at timestamp; the server keeps the latest snapshot per user
-- so the dashboard can render "you currently own these ships".
--
-- Storage shape:
--   * One row per user. Replacing the row on every push is intentional --
--     hangar history is not a feature here, just current state. The
--     server stamps `captured_at` server-side rather than trusting the
--     client's clock.
--   * `ships` is JSONB so the wire shape can grow without schema churn
--     (the only stable fields are `name` + the optional metadata).
--
-- The tray client de-duplicates: it won't POST a hangar identical to
-- the most recent snapshot. The server still upserts unconditionally
-- so a manual "force refresh" from the UI works.

CREATE TABLE IF NOT EXISTS hangar_snapshots (
    user_id     UUID        PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    captured_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    ships       JSONB       NOT NULL DEFAULT '[]'::jsonb
);
