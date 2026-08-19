-- 0051_parser_inference_rules.sql -- Published inference rules served in the manifest.
--
-- The runtime manifest (GET /v1/parser-definitions) serves inference rules
-- (starstats-core::inference_defs::RemoteInferenceRule) from this table.
-- The whole rule is stored as JSONB (`definition`) because it is nested
-- (trigger + followups[] + emits) — a flat-column mapping would be lossy.
-- Retract by flipping `enabled` FALSE (the manifest publishes by absence).
-- Additive; no FK.
CREATE TABLE IF NOT EXISTS parser_inference_rules (
    rule_id      TEXT        PRIMARY KEY,
    definition   JSONB       NOT NULL,
    enabled      BOOLEAN     NOT NULL DEFAULT TRUE,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS parser_inference_rules_enabled_idx
    ON parser_inference_rules (rule_id) WHERE enabled;
