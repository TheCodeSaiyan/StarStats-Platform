-- Per-capability enforcement for account misuse.
--
-- Absence of a row IS the unrestricted state. Lifting a restriction
-- DELETES the row rather than writing all-false flags, so there is
-- exactly one way to spell "unrestricted" and the guard has one case
-- to handle.
--
-- Keyed by user_id, NOT claimed_handle. The request guard reads the
-- user id straight off the JWT `sub`, so the hot path never touches a
-- handle and cannot be defeated by handle casing (stat_event_counts
-- already had to grow a lowercase CHECK for exactly that reason). The
-- one place a handle join is needed -- filtering public profiles -- does
-- the LOWER() normalisation in a single query.
--
-- `expires_at` is evaluated at READ time (`expires_at IS NULL OR
-- expires_at > now()`), never by a sweep job. A sweep that silently
-- stopped running would un-suspend everyone, and a stuck background job
-- is invisible until someone goes looking.

CREATE TABLE IF NOT EXISTS account_restrictions (
    user_id                UUID        PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    ingest_blocked         BOOLEAN     NOT NULL DEFAULT FALSE,
    sharing_blocked        BOOLEAN     NOT NULL DEFAULT FALSE,
    public_profile_blocked BOOLEAN     NOT NULL DEFAULT FALSE,
    submissions_blocked    BOOLEAN     NOT NULL DEFAULT FALSE,
    -- Required. A restriction with no stated reason is unreviewable
    -- later, and the reason is surfaced to the restricted user.
    reason                 TEXT        NOT NULL,
    -- Moderator handle, for the audit trail and the admin UI.
    restricted_by          TEXT        NOT NULL,
    restricted_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    -- NULL means "until lifted".
    expires_at             TIMESTAMPTZ NULL
);

-- Serves the public-profile read filter, which joins users ->
-- account_restrictions for a batch of handles and only cares about
-- rows that block public profiles.
CREATE INDEX IF NOT EXISTS account_restrictions_public_blocked_idx
    ON account_restrictions (user_id)
    WHERE public_profile_blocked;
