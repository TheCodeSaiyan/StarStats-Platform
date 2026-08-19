-- 0010_login_factors.sql -- Magic-link login + TOTP 2FA + recovery codes.
--
-- Three independent capabilities, packaged together because they all
-- shape what "logging in" can look like and ship in the same wave:
--
-- 1. Magic-link login. The user submits their email, we mail them a
--    one-shot link, clicking it mints a session JWT. Same UX as the
--    password reset flow but the redemption replaces the password
--    check entirely instead of bumping the hash.
--
-- 2. TOTP (RFC 6238). User pairs their account with an authenticator
--    app; the shared secret is AES-256-GCM-encrypted at rest with a
--    KEK held in a server-local file (analogous to the JWT signing
--    key). Login flow grows a second leg: password ok -> interim
--    "totp_required" token -> TOTP code -> full session JWT.
--
-- 3. Recovery codes. 10 single-use codes generated alongside TOTP
--    activation. Argon2-hashed (one-way) — we don't need to display
--    them again, only verify them when a user submits one. Each row
--    has its own `used_at` so we never accept the same code twice.

-- Magic-link tokens. Sparse — most users never use this flow, so we
-- keep the table small and let it accumulate. Cleanup is a follow-up
-- (job that deletes expired+consumed rows older than 24h); not worth
-- adding now while the user count is single-digit.
CREATE TABLE IF NOT EXISTS magic_link_tokens (
    token       TEXT PRIMARY KEY,
    user_id     UUID        NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    expires_at  TIMESTAMPTZ NOT NULL,
    consumed_at TIMESTAMPTZ NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS magic_link_tokens_user_id_idx
    ON magic_link_tokens (user_id);

-- TOTP fields on users.
--
-- `totp_secret_ciphertext` + `totp_secret_nonce` are written together
-- by the AEAD encrypt step. The nonce is 96 bits (12 bytes) per
-- AES-GCM; we generate a fresh nonce on every encrypt rather than
-- attempting to reuse one (nonce reuse with the same key is
-- catastrophic for GCM, so the safe pattern is "always fresh").
--
-- `totp_setup_at` distinguishes "key issued, awaiting first valid
-- code" from `totp_enabled_at`, "user has proven the secret works."
-- Login enforcement keys off `totp_enabled_at` only — a user who
-- starts setup but doesn't complete it stays in single-factor.
ALTER TABLE users
    ADD COLUMN IF NOT EXISTS totp_secret_ciphertext BYTEA       NULL,
    ADD COLUMN IF NOT EXISTS totp_secret_nonce      BYTEA       NULL,
    ADD COLUMN IF NOT EXISTS totp_setup_at          TIMESTAMPTZ NULL,
    ADD COLUMN IF NOT EXISTS totp_enabled_at        TIMESTAMPTZ NULL;

-- Recovery codes. Hashed, not encrypted: we never need to recover
-- them, only verify-and-burn. A regenerate operation deletes the
-- whole set and inserts a fresh batch.
CREATE TABLE IF NOT EXISTS recovery_codes (
    id         UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id    UUID        NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    code_hash  TEXT        NOT NULL,
    used_at    TIMESTAMPTZ NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS recovery_codes_user_id_idx
    ON recovery_codes (user_id);
