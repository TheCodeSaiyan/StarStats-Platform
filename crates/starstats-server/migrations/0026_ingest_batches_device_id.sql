-- 0026_ingest_batches_device_id.sql — Per-device ingest batch filter.
--
-- The Devices page (W2 redesign) renders an Activity tab per paired
-- desktop client. To filter the batch list to just that device, the
-- ingest handler now stamps the device_id (read off the bearer
-- token's JWT claim) into the audit_log payload for the canonical
-- ingest action.
--
-- There is no `ingest_batches` table — ingest history is reconstructed
-- from `audit_log` rows where `action = 'ingest.batch_processed'`.
-- audit_log is hash-chained and append-only, so we deliberately do
-- NOT add a column to that table: the hash chain covers payload only
-- and adding a new column would invite drift between the canonical
-- payload field and an out-of-band column. Instead device_id lives in
-- the payload JSONB alongside batch_id/total/accepted/etc — the same
-- pattern every other ingest field follows.
--
-- This migration is the index half: a partial functional index that
-- the new `?device_id=<uuid>` read path on /v1/me/ingest-history hits
-- when filtering one device's batches out of the account-wide stream.
--
-- Existing rows have no device_id in their payload (the column is
-- absent rather than NULL — `->>'device_id'` returns NULL either way).
-- Backfill is not in scope; legacy rows simply won't match a
-- device-scoped filter and the UI presents them under "all activity".

CREATE INDEX IF NOT EXISTS audit_log_ingest_device_idx
    ON audit_log (actor_handle, ((payload->>'device_id')), occurred_at DESC)
    WHERE action = 'ingest.batch_processed';
