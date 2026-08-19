-- 0036_devices_sync_enabled.sql — per-device opt-in gate for cloud sync.
--
-- Default FALSE so existing paired devices stay sync-disabled on
-- deploy. Users opt in deliberately (per spec §1 non-goal "no
-- migration backfill") via the tray's local Cloud sync toggle or
-- from the Connected Uplinks page on the web.
--
-- Additive only — matches the migration invariants in docs/ENGINEERING.md
-- (IF NOT EXISTS, NULL/DEFAULT, no DROPs).

ALTER TABLE devices
    ADD COLUMN IF NOT EXISTS sync_enabled BOOLEAN NOT NULL DEFAULT FALSE;
