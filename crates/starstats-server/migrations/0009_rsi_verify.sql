-- 0009_rsi_verify.sql -- RSI handle ownership verification.
--
-- StarStats trusts the user-supplied `claimed_handle` at signup time
-- only as far as "you say you are this person." That's fine for
-- private accounts, but the moment a profile goes public (or is
-- shared with an org) the claim becomes a *representation* about
-- another identity — the RSI account named `claimed_handle`. Without
-- proof, anyone could sign up as `TheCodeSaiyan` and publish stats
-- under that name.
--
-- The verification flow mirrors what other Star Citizen tooling
-- (RSI.tools, Erkul, etc.) does:
--
--   1. We issue a short, distinctive code (`STARSTATS-XXXXXXXX`).
--   2. The user pastes it into the bio field of their RSI public
--      profile at robertsspaceindustries.com/citizens/{handle}.
--   3. We fetch the profile page and look for the code in the body.
--      If present, we mark the user verified, clear the code, and
--      they can take their bio back.
--
-- `rsi_verified_at` is a timestamp rather than a bool so we can
-- record *when* verification last happened (in case we ever need to
-- re-verify after a stale period — current policy is "verify once,
-- trust forever," but the column gives us a future hook).
--
-- The token-shaped columns are nullable: most users won't have a
-- verification in flight at any given moment. The partial index
-- keeps the lookup O(1) while leaving 99% of rows out of it.

ALTER TABLE users
    ADD COLUMN IF NOT EXISTS rsi_verified_at        TIMESTAMPTZ NULL,
    ADD COLUMN IF NOT EXISTS rsi_verify_code        TEXT        NULL,
    ADD COLUMN IF NOT EXISTS rsi_verify_expires_at  TIMESTAMPTZ NULL;

CREATE INDEX IF NOT EXISTS users_rsi_verify_code_idx
    ON users (rsi_verify_code)
    WHERE rsi_verify_code IS NOT NULL;
