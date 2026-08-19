-- Contracts ingested from sp-ingest (the StarPlatform capture tool).
--
-- sp-ingest POSTs a `PublishBundle` to `/api/contracts/ingest`; the
-- server UPSERTs one row per `canonical_id` (the sender deliberately
-- folds multi-source scans into a single canonical id, so a repeat
-- push is the SAME contract — never a duplicate).
--
-- Hybrid storage:
--   * promoted scalar columns drive list filters + search,
--   * `record` JSONB holds the FULL internal packet verbatim
--     (source_capture_id + raw_text + extraction + suggestion +
--     confidence_score + flags) so nothing the sender shipped is lost.
--
-- `raw_text` lives inside `record` only and is NEVER surfaced by the
-- public read DTOs — the read layer projects the structured extraction,
-- it does not echo the JSONB blob.
--
-- Additive-only + byte-immutable post-deploy (see docs/ENGINEERING.md
-- "Migrations are additive only AND byte-immutable post-deploy").
CREATE TABLE IF NOT EXISTS contracts (
    -- Upsert key. The sender's stable machine id for the contract.
    canonical_id      TEXT PRIMARY KEY,
    -- Payload schema version the row was ingested under (PublishBundle
    -- `schema_version`). Stored so a future major can branch on it.
    schema_version    TEXT NOT NULL,
    -- Most-recent capture that produced this canonical contract.
    capture_id        TEXT,
    -- Promoted, queryable projection of the extracted contract. All
    -- nullable because almost every field on the wire is nullable and
    -- `contract_type` / `legal_status` are open strings (no hard enum).
    display_name      TEXT,
    contract_type     TEXT,
    subcategory       TEXT,
    gameplay_loop     TEXT,
    issuer            TEXT,
    faction           TEXT,
    legal_status      TEXT,
    reward_amount     BIGINT,
    reward_currency   TEXT,
    patch_version     TEXT,
    confidence_score  DOUBLE PRECISION,
    -- Sender's advisory intent (create_new_contract | update_existing_contract
    -- | duplicate | patch_change | outdated_removed | partial_capture_review
    -- | low_confidence). We still upsert by canonical_id regardless.
    suggested_action  TEXT,
    -- Lowercased, space-joined bag of searchable terms (display_name,
    -- issuer, type, subcategory, gameplay_loop, faction, attribute
    -- values, step locations, primary objectives). Drives ?q= / ?location=
    -- ILIKE search without a full-text extension dependency.
    search_blob       TEXT NOT NULL DEFAULT '',
    -- Full internal AdminReviewPacket as received. Source of truth.
    record            JSONB NOT NULL,
    -- First ingest wins first_seen_at; every upsert bumps updated_at.
    first_seen_at     TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS contracts_type_idx ON contracts (contract_type);
CREATE INDEX IF NOT EXISTS contracts_issuer_lower_idx ON contracts (LOWER(issuer));
CREATE INDEX IF NOT EXISTS contracts_updated_at_idx ON contracts (updated_at DESC);
