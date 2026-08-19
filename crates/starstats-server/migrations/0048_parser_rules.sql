-- 0048_parser_rules.sql -- Published parser rules served in the manifest.
--
-- The runtime parser-definition manifest (GET /v1/parser-definitions,
-- fetched by the tray) is served from this table instead of the former
-- hardcoded-empty stub. Each row is one approved `RemoteRule`
-- (starstats-core::parser_defs::RemoteRule). A rule is retracted by
-- flipping `enabled` to FALSE (the manifest publishes by absence).
--
-- Decoupled from `parser_submissions` on purpose: one rule can cover
-- many submitted shapes, and the served set shouldn't inherit the
-- churn/noise of the triage table. The approval flow (a follow-up
-- slice) upserts into here when a submission becomes a real rule.
--
-- `match_kind` mirrors the core enum's snake_case TEXT ('event_name' |
-- 'body_keyword'). `fields` is the JSON array of capture names to
-- surface. Additive; no FK.

CREATE TABLE IF NOT EXISTS parser_rules (
    rule_id      TEXT        PRIMARY KEY,
    event_name   TEXT        NOT NULL,
    match_kind   TEXT        NOT NULL DEFAULT 'event_name',
    body_regex   TEXT        NOT NULL,
    fields       JSONB       NOT NULL DEFAULT '[]'::jsonb,
    enabled      BOOLEAN     NOT NULL DEFAULT TRUE,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- The manifest read is `WHERE enabled ORDER BY rule_id`; a partial
-- index keeps it cheap as retracted rules accumulate.
CREATE INDEX IF NOT EXISTS parser_rules_enabled_idx
    ON parser_rules (rule_id) WHERE enabled;
