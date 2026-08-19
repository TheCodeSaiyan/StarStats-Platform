-- 0002_audit_log.sql -- Append-only, hash-chained audit log.
--
-- Trust model: every state-changing API call writes one row here.
-- Each row's hash covers (prev_hash || canonical_payload). A single
-- byte change in any historical row breaks the chain — we can detect
-- tampering by walking from any verified anchor forward.
--
-- Append-only is enforced at the database level by:
--  * a row-level trigger that rejects UPDATE/DELETE on this table,
--  * REVOKE UPDATE, DELETE on the table from non-superusers (granted
--    in deploy via psql; cannot express here in a portable migration).
--
-- Hash:
--   prev_hash = previous row's row_hash (or 32 zero bytes for the seed)
--   row_hash  = SHA-256(prev_hash || jsonb_canonical(payload) || seq::text)
-- The application computes row_hash and writes it; the trigger
-- verifies the link before commit.

CREATE TABLE IF NOT EXISTS audit_log (
    seq         BIGSERIAL   PRIMARY KEY,
    occurred_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    actor_sub   TEXT,                                -- token sub, NULL for system
    actor_handle TEXT,                               -- preferred_username when known
    action      TEXT        NOT NULL,                -- e.g. 'ingest.batch_accepted'
    payload     JSONB       NOT NULL,
    prev_hash   BYTEA       NOT NULL,
    row_hash    BYTEA       NOT NULL UNIQUE,
    CHECK (octet_length(prev_hash) = 32),
    CHECK (octet_length(row_hash)  = 32)
);

CREATE INDEX IF NOT EXISTS audit_log_actor_idx
    ON audit_log (actor_sub, occurred_at DESC) WHERE actor_sub IS NOT NULL;

CREATE INDEX IF NOT EXISTS audit_log_action_idx
    ON audit_log (action, occurred_at DESC);

-- Reject any UPDATE or DELETE — audit_log is append-only.
CREATE OR REPLACE FUNCTION audit_log_no_mutate() RETURNS trigger AS $$
BEGIN
    RAISE EXCEPTION 'audit_log is append-only (op=%)', TG_OP;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS audit_log_no_update ON audit_log;
CREATE TRIGGER audit_log_no_update
    BEFORE UPDATE ON audit_log
    FOR EACH ROW EXECUTE FUNCTION audit_log_no_mutate();

DROP TRIGGER IF EXISTS audit_log_no_delete ON audit_log;
CREATE TRIGGER audit_log_no_delete
    BEFORE DELETE ON audit_log
    FOR EACH ROW EXECUTE FUNCTION audit_log_no_mutate();

-- Verify the hash chain on insert: prev_hash must match the previous
-- row's row_hash (or be all zeros if this is the very first row).
CREATE OR REPLACE FUNCTION audit_log_check_chain() RETURNS trigger AS $$
DECLARE
    expected BYTEA;
BEGIN
    SELECT row_hash INTO expected
      FROM audit_log
      ORDER BY seq DESC
      LIMIT 1;

    IF expected IS NULL THEN
        expected := decode(repeat('00', 32), 'hex');
    END IF;

    IF NEW.prev_hash <> expected THEN
        RAISE EXCEPTION 'audit_log chain break: prev_hash does not match prior row_hash';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS audit_log_chain_check ON audit_log;
CREATE TRIGGER audit_log_chain_check
    BEFORE INSERT ON audit_log
    FOR EACH ROW EXECUTE FUNCTION audit_log_check_chain();
