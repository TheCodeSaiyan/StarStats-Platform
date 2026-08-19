-- 0014_rsi_orgs.sql — RSI org-membership snapshot.
--
-- Periodic snapshot of which Star Citizen organisations the user
-- belongs to (main + affiliations), scraped from
-- https://robertsspaceindustries.com/citizens/{handle}/organizations.
-- Public page — same posture as `rsi_profile_snapshots`.
--
-- Storage shape mirrors `hangar_snapshots`: one row per user, full
-- list replaced wholesale on every refresh. No history is kept here;
-- if "show me when I left org X" becomes a feature it gets its own
-- append-only table.

CREATE TABLE IF NOT EXISTS rsi_org_snapshots (
    user_id     UUID        PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    captured_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    orgs        JSONB       NOT NULL DEFAULT '[]'::jsonb
);
