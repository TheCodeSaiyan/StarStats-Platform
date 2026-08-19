-- 0008_auth_email_flows.sql -- Password reset + email change flows.
--
-- Adds three independent capabilities to the `users` table:
--
-- 1. Password reset. The user requests a one-time token that is
--    emailed to them; submitting it with a new password rotates the
--    hash and bumps `password_changed_at` (which the JWT verifier
--    consults to invalidate sessions issued before the change).
--
-- 2. Email change. A logged-in user can stage a new address; the
--    address is held in `pending_email` until they click the
--    verification link. The old address remains the login until the
--    new one is confirmed — protects against typos locking accounts
--    out.
--
-- 3. Token-based session invalidation. `password_changed_at` is set
--    to NOW() at user creation and bumped on every password change.
--    The auth verifier rejects tokens whose `iat` claim predates this
--    column (after a small skew tolerance, see auth.rs). Without this
--    column we have no way to revoke user JWTs short of rotating the
--    server-wide signing key.
--
-- All new columns are nullable except `password_changed_at`, which
-- defaults to NOW() so existing rows get a sensible baseline (any
-- token issued before the migration is effectively invalidated, which
-- is the safe posture).

ALTER TABLE users
    ADD COLUMN IF NOT EXISTS password_reset_token       TEXT        NULL,
    ADD COLUMN IF NOT EXISTS password_reset_expires_at  TIMESTAMPTZ NULL,
    ADD COLUMN IF NOT EXISTS password_changed_at        TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    ADD COLUMN IF NOT EXISTS pending_email              TEXT        NULL,
    ADD COLUMN IF NOT EXISTS pending_email_token        TEXT        NULL,
    ADD COLUMN IF NOT EXISTS pending_email_expires_at   TIMESTAMPTZ NULL;

-- Lookups by reset token are sparse and have to be O(1). Partial
-- index keeps the index tiny — most rows have no token in flight.
CREATE INDEX IF NOT EXISTS users_password_reset_token_idx
    ON users (password_reset_token)
    WHERE password_reset_token IS NOT NULL;

CREATE INDEX IF NOT EXISTS users_pending_email_token_idx
    ON users (pending_email_token)
    WHERE pending_email_token IS NOT NULL;
