-- 0004_users.sql -- User accounts for self-hosted auth.
--
-- Identity model:
--  * `id` is the JWT `sub` claim — UUIDv4, opaque, never recycled.
--  * `email` is unique, case-insensitive (we lowercase on write).
--    Used as the login identifier; address validation happens in the
--    application layer (cheap regex, not a DNS check).
--  * `password_hash` is an Argon2id PHC string. Verifier reads the
--    parameters out of the string itself, so increasing argon2 cost
--    is just a re-hash on next successful login (Slice 4 follow-up).
--  * `claimed_handle` is the RSI handle the user is asserting. We
--    do NOT verify it on signup — that happens via the OAuth link
--    flow in Slice 5 (RSI doesn't expose a public verification API,
--    so cross-checks against community SSO are the practical path).
--    Until then, the handle is self-claimed and the audit trail
--    flags any mismatched ingest at `claimed_handle != preferred_username`.

CREATE TABLE IF NOT EXISTS users (
    id              UUID        PRIMARY KEY,
    email           TEXT        NOT NULL,
    password_hash   TEXT        NOT NULL,
    claimed_handle  TEXT        NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Lowercased uniqueness — the app writes `lower(email)` but we still
-- guard with a functional index in case a future code path forgets.
CREATE UNIQUE INDEX IF NOT EXISTS users_email_uq
    ON users (lower(email));

-- Handles are case-insensitive in RSI. Two accounts can't both claim
-- "TheCodeSaiyan" / "thecodesaiyan".
CREATE UNIQUE INDEX IF NOT EXISTS users_handle_uq
    ON users (lower(claimed_handle));
