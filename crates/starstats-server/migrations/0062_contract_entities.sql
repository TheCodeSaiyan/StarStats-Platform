-- Which KB entities each contract references, derived at ingest.
--
-- Answers both link directions from one indexed source: "what does this
-- contract touch" (the contract detail page) and "which contracts touch
-- this" (a KB entity page). Before this table the first question was
-- answered by fetching a reference catalogue per page render — the
-- vehicles bundle alone is ~4 MB — and the second could not be answered
-- at all except by substring-matching `search_blob`.
--
-- DERIVED, NOT AUTHORITATIVE. Every row is reproducible by re-publishing
-- the contract, which is why there is no backfill: the catalogue is
-- republished from captures as a routine workflow.
--
-- `ref_slug` / `ref_category` are NULL whenever resolution was not
-- unambiguous — no match, several matches, or a matched registry row
-- whose own slug is still NULL from before migration 0038. The row is
-- still written in that case, because `raw_value` is what the surface
-- renders as plain text, and an unresolved entity is still evidence the
-- contract touches that place or item.
CREATE TABLE IF NOT EXISTS contract_entities (
    canonical_id  TEXT NOT NULL REFERENCES contracts(canonical_id) ON DELETE CASCADE,
    -- 'location' | 'item' | 'vehicle' | 'weapon'. Open string rather than
    -- an enum: the extractor gains kinds before the schema should.
    kind          TEXT NOT NULL,
    raw_value     TEXT NOT NULL,
    -- lower-cased, trimmed, internal whitespace collapsed. Part of the
    -- primary key so two spellings differing only in spacing collapse to
    -- one row rather than double-counting the entity.
    value_norm    TEXT NOT NULL,
    ref_slug      TEXT,
    ref_category  TEXT,
    PRIMARY KEY (canonical_id, kind, value_norm)
);

-- Drives the KB entity page: "which contracts reference this slug".
CREATE INDEX IF NOT EXISTS contract_entities_ref_idx
    ON contract_entities (ref_category, ref_slug);

-- Drives lookup by raw name for entities the registry could not resolve.
CREATE INDEX IF NOT EXISTS contract_entities_norm_idx
    ON contract_entities (kind, value_norm);
