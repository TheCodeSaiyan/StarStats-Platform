-- 0020_smtp_config.sql -- DB-backed SMTP configuration with KEK-
-- encrypted password.
--
-- Singleton table: enforced by the CHECK on id. Only one row ever
-- exists, regardless of insert intent. The row is seeded as disabled
-- by this migration so the loader can do a straight SELECT instead of
-- branching on "row absent". Updates target id = 1 and use UPSERT
-- semantics in code.
--
-- Password lives in two columns: `password_ciphertext` is the result
-- of `Kek::encrypt(plaintext)` and `password_nonce` is its second
-- tuple element (the per-row nonce). Both are NULL when no password
-- has ever been set, which is a valid state for SMTP servers that
-- accept unauthenticated relay on a private network. The decrypt
-- path treats `NULL` ciphertext as "no auth".
--
-- This migration does NOT remove the `SMTP_*` env vars from boot-time
-- config — the loader prefers a DB row that has `enabled = true` and
-- falls back to env otherwise, so the first deploy of this migration
-- doesn't break an existing env-driven install.

CREATE TABLE IF NOT EXISTS smtp_config (
    id                  INT          PRIMARY KEY CHECK (id = 1),
    host                TEXT         NOT NULL DEFAULT '',
    port                INT          NOT NULL DEFAULT 587,
    username            TEXT         NOT NULL DEFAULT '',
    password_ciphertext BYTEA        NULL,
    password_nonce      BYTEA        NULL,
    secure              BOOLEAN      NOT NULL DEFAULT TRUE,
    from_addr           TEXT         NOT NULL DEFAULT 'noreply@starstats.local',
    from_name           TEXT         NOT NULL DEFAULT 'StarStats',
    web_origin          TEXT         NOT NULL DEFAULT '',
    enabled             BOOLEAN      NOT NULL DEFAULT FALSE,
    updated_at          TIMESTAMPTZ  NOT NULL DEFAULT now(),
    updated_by          UUID         NULL REFERENCES users(id) ON DELETE SET NULL,
    -- Symmetry with the ciphertext columns: either both are set, or
    -- both are NULL. Mismatched halves indicate corruption.
    CONSTRAINT smtp_config_password_pair CHECK (
        (password_ciphertext IS NULL AND password_nonce IS NULL)
        OR (password_ciphertext IS NOT NULL AND password_nonce IS NOT NULL)
    )
);

-- Seed the singleton row. Subsequent runs are no-ops thanks to the
-- ON CONFLICT clause; the migration is safe to re-apply.
INSERT INTO smtp_config (id) VALUES (1)
    ON CONFLICT (id) DO NOTHING;
