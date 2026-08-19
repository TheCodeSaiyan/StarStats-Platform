-- 0016_submissions.sql -- Community-curated parser-rule submissions.
--
-- Three tables, all keyed on `users.id` so per-user "once per item"
-- semantics fall out of the primary key (no race window between a
-- read-and-write toggle).
--
-- Status lifecycle:
--   review   ──► accepted ──► shipped
--                │
--                └─► rejected (terminal)
--   review   ──► withdrawn (submitter only, terminal)
--   review   ──► flagged   (auto-routed at >= AUTO_FLAG_THRESHOLD distinct flaggers)
--
-- The flagged transition is computed in the application layer because
-- it depends on a count over `submission_flags`; the lifecycle from
-- there onwards (accept / reject / dismiss flag) is moderator action,
-- which lands in a later wave once a moderator UI exists.

CREATE TABLE IF NOT EXISTS submissions (
    id              UUID        PRIMARY KEY,
    submitter_id    UUID        NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    -- Raw match pattern as the user wrote it. Free-form text rather
    -- than a structured regex schema; the moderator translates to a
    -- real parser rule before acceptance.
    pattern         TEXT        NOT NULL,
    -- Proposed event_type slug. snake_case ASCII; moderator may rename
    -- on acceptance. Validated at the API boundary, not in the DB.
    proposed_label  TEXT        NOT NULL,
    description     TEXT        NOT NULL,
    -- One example raw line that the pattern caught — used to make the
    -- submission concrete for reviewers.
    sample_line     TEXT        NOT NULL,
    -- live / ptu / eptu — where the submitter saw the line.
    log_source      TEXT        NOT NULL,
    status          TEXT        NOT NULL DEFAULT 'review',
    rejection_reason TEXT,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CHECK (status IN ('review', 'accepted', 'shipped', 'rejected', 'withdrawn', 'flagged'))
);

CREATE INDEX IF NOT EXISTS submissions_submitter_idx
    ON submissions (submitter_id, created_at DESC);

CREATE INDEX IF NOT EXISTS submissions_status_idx
    ON submissions (status, created_at DESC);

CREATE TABLE IF NOT EXISTS submission_votes (
    submission_id   UUID        NOT NULL REFERENCES submissions(id) ON DELETE CASCADE,
    user_id         UUID        NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (submission_id, user_id)
);

CREATE INDEX IF NOT EXISTS submission_votes_user_idx
    ON submission_votes (user_id, created_at DESC);

CREATE TABLE IF NOT EXISTS submission_flags (
    submission_id   UUID        NOT NULL REFERENCES submissions(id) ON DELETE CASCADE,
    user_id         UUID        NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    -- Free-text reason; capped at the API layer. Optional because
    -- "this looks broken" without a body is still a useful signal.
    reason          TEXT,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (submission_id, user_id)
);

CREATE INDEX IF NOT EXISTS submission_flags_user_idx
    ON submission_flags (user_id, created_at DESC);
