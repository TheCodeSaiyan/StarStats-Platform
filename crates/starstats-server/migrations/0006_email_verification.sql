-- 0006_email_verification.sql -- Email verification columns on users.
--
-- Adds three columns and a partial unique index:
--
--  * `email_verified_at` — set when the user clicks the link in
--    their verification email. NULL means the address is unverified;
--    we currently allow login but future auth-flow gates can refuse
--    privileged actions until this is set.
--
--  * `email_verification_token` — opaque random token (32 bytes hex,
--    so 64 hex chars) emailed to the user at signup. Cleared after
--    successful verification so a token can't be replayed.
--
--  * `email_verification_expires_at` — token TTL. We default to 24h
--    in application code; a stale token is a "request a new one"
--    flow rather than an automatic resend.
--
-- The unique index is partial — multiple users sit at NULL token
-- but no two users may share a live (non-NULL) token. This keeps
-- token lookup an indexed equality scan.

ALTER TABLE users ADD COLUMN email_verified_at TIMESTAMPTZ NULL;
ALTER TABLE users ADD COLUMN email_verification_token TEXT NULL;
ALTER TABLE users ADD COLUMN email_verification_expires_at TIMESTAMPTZ NULL;

CREATE UNIQUE INDEX users_verification_token_uq
    ON users (email_verification_token)
    WHERE email_verification_token IS NOT NULL;
