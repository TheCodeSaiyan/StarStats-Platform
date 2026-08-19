-- 0029_parser_submissions.sql -- Tray-promoted unknown-line submissions.
--
-- The tray classifies a `Game.log` line as unknown when no built-in
-- parser nor remote rule matches it. The user reviews redaction and
-- opts in to submit; this table holds the result so a rule author can
-- propose a parser rule from real data.
--
-- Identity is `(shape_hash, client_anon_id)`: repeated submissions
-- from the same install fold into a single row with bumped occurrence
-- totals; distinct installs each get their own row so we can count
-- *distinct submitters per shape* (a stronger signal than the single
-- install's occurrence count). Aggregation across `shape_hash` is a
-- read-side concern, not a write-side one.
--
-- `payload_json` carries the full `ParserSubmission` wire form for
-- replay against future tooling — we keep the schema thin at the
-- column level so backward-compat parsing changes never need an ALTER.

CREATE TABLE IF NOT EXISTS parser_submissions (
    id                      BIGSERIAL   PRIMARY KEY,
    shape_hash              TEXT        NOT NULL,
    client_anon_id          TEXT        NOT NULL,
    first_submitted_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_submitted_at       TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    submitter_count         INTEGER     NOT NULL DEFAULT 1,
    total_occurrence_count  INTEGER     NOT NULL DEFAULT 0,
    payload_json            JSONB       NOT NULL,
    status                  TEXT        NOT NULL DEFAULT 'pending',
    reviewer_notes          TEXT,
    rule_id                 TEXT,
    UNIQUE (shape_hash, client_anon_id)
);

CREATE INDEX IF NOT EXISTS parser_submissions_shape
    ON parser_submissions(shape_hash);

CREATE INDEX IF NOT EXISTS parser_submissions_status
    ON parser_submissions(status);
