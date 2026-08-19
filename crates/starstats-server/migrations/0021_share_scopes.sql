-- 0021_share_scopes.sql -- Per-user share-scope policy for the
-- metrics-redesign aggregate endpoints.
--
-- Each aggregate endpoint consults the target user's `share_scopes`
-- before returning data. Three exits: 'me' (owner only), 'friend'
-- (friend-scoped via the existing friend system), 'public' (anyone).
--
-- The 'leaderboards' key is present from day one even though org
-- leaderboards don't ship in v0.0.2-beta — declaring the shape now
-- means a future Phase 5 doesn't need to retro-rewrite Phase 1
-- storage.
--
-- Defaults are intentionally conservative: only the legacy 'summary'
-- surface defaults to 'friend' (mirroring current behaviour), every
-- other surface defaults to 'me' so opting out is the default.

ALTER TABLE users
  ADD COLUMN IF NOT EXISTS share_scopes JSONB NOT NULL DEFAULT '{
    "summary": "friend",
    "sessions": "me",
    "deaths": "me",
    "shards": "me",
    "leaderboards": "me"
  }'::jsonb;
