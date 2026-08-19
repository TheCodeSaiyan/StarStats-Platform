-- 0036_retention_policies.sql
--
-- Tier-based data retention policy table. Drives the per-user events
-- purge loop spawned by main.rs (see src/retention.rs).
--
-- Tier is derived at read time from `supporter_status.state`
-- (`active` -> supporter, anything else -> free). This table maps
-- the tier name to a retention window expressed in days.
--
-- `retention_days IS NULL` means "unlimited retention" (no purge for
-- this tier). The job loop short-circuits on NULL so a supporter row
-- never even reaches the DELETE path. Migration 0017's docstring
-- anticipates this: lapsed users keep the pill + name_plate but the
-- retention extension reverts -- which is exactly what `free=90`
-- expresses below.
--
-- Per docs/ENGINEERING.md invariants: additive only, IF NOT EXISTS, no DROPs,
-- byte-immutable post-deploy. Tunable without a code deploy via
-- UPDATE on this table; the purge loop re-reads policies on each
-- sweep so a change takes effect on the next tick.

CREATE TABLE IF NOT EXISTS retention_policies (
    tier            TEXT        PRIMARY KEY,
    retention_days  INTEGER,
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CHECK (retention_days IS NULL OR retention_days > 0)
);

-- Seed the two known tiers. ON CONFLICT DO NOTHING so a re-run on an
-- environment where an operator has already tuned the values is a
-- no-op (the operator's tuning wins).
INSERT INTO retention_policies (tier, retention_days) VALUES
    ('free', 90),
    ('supporter', NULL)
ON CONFLICT (tier) DO NOTHING;
