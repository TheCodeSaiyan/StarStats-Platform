-- 0015_user_preferences.sql — per-user UI preferences (theme + future).
--
-- Stored as a JSONB column on `users` rather than a separate table
-- because the field set is small and forward-extensible: theme today,
-- notification toggles + name plate (supporter, 28-char cap) + accent
-- intensity tomorrow. Defaults to empty object; `theme` falls back
-- to `stanton` server-side when absent.
--
-- Theme allowlist enforced at the application layer
-- (preferences_routes.rs) — Postgres-side CHECK constraint would break
-- forward compat when the UI ships a fifth theme.

ALTER TABLE users
    ADD COLUMN IF NOT EXISTS preferences JSONB NOT NULL DEFAULT '{}'::jsonb;
