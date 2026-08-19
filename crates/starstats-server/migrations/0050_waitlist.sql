-- 0050_waitlist.sql -- Public-beta signup gate.
--
-- Two tables. `waitlist_signups` is the queue: one row per email that
-- asked in. `waitlist_config` is a singleton (id = 1, enforced by CHECK)
-- holding the admission cap and the gate switch, mirroring
-- 0020_smtp_config and 0042_ship_matrix_config -- so the cap can move at
-- 2am from the admin UI with no redeploy.
--
-- `admitted_at IS NULL` means queued; non-NULL means admitted and an
-- invite was minted. `invite_token` is the bearer of that admission: it
-- is what /v1/auth/signup checks while the gate is on. It is consumed
-- (invite_consumed_at set) on successful signup so a forwarded email
-- cannot mint a second account -- and released back if the signup then
-- fails, so a taken handle does not eject someone from the beta.
--
-- `source` is free-text attribution ("reddit", "spectrum", ...) so the
-- launch can tell which channel actually sent people. Nullable: a signup
-- with no attribution is still a signup.
--
-- Additive + byte-immutable post-deploy: IF NOT EXISTS, NOT NULL with
-- defaults, seed row via ON CONFLICT DO NOTHING.

CREATE TABLE IF NOT EXISTS waitlist_signups (
    id                  UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    -- Stored lowercase. Handle-case drift already split one live user's
    -- data across two cases (fixed in 0044/0045); do not re-learn that
    -- lesson on emails.
    email               TEXT        NOT NULL UNIQUE,
    source              TEXT        NULL,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    admitted_at         TIMESTAMPTZ NULL,
    invite_token        TEXT        NULL UNIQUE,
    invite_consumed_at  TIMESTAMPTZ NULL
);

-- The queue read: oldest-first among the not-yet-admitted.
CREATE INDEX IF NOT EXISTS waitlist_signups_queue_idx
    ON waitlist_signups (created_at)
    WHERE admitted_at IS NULL;

-- The cap check counts admitted rows on every signup.
CREATE INDEX IF NOT EXISTS waitlist_signups_admitted_idx
    ON waitlist_signups (admitted_at)
    WHERE admitted_at IS NOT NULL;

CREATE TABLE IF NOT EXISTS waitlist_config (
    id           INT         PRIMARY KEY CHECK (id = 1),
    -- Admission cap. Auto-admit while COUNT(admitted) < cap. Sized by
    -- maintainer capacity (bug reports absorbed per week), not by
    -- infrastructure -- the server would not notice 50 users.
    cap          INT         NOT NULL DEFAULT 50,
    -- The gate itself. FALSE = signup is open to anyone and invite tokens
    -- are not required, i.e. today's behaviour. Default FALSE so deploying
    -- this migration changes nothing until an admin turns it on, and so
    -- turning it back off is the rollback -- no redeploy either way.
    gate_enabled BOOLEAN     NOT NULL DEFAULT FALSE,
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_by   UUID        NULL REFERENCES users(id) ON DELETE SET NULL
);

INSERT INTO waitlist_config (id) VALUES (1) ON CONFLICT (id) DO NOTHING;
