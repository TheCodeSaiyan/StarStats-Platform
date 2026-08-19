-- 0017_supporter_status.sql -- Per-user supporter (donate) status.
--
-- Backs Wave 9 (Donate via Revolut Business). The actual payment-flow
-- wiring lives in a follow-up commit; this migration just lands the
-- data shape so the read endpoint, profile pill, and any manual data
-- fix the operator wants to apply have somewhere to live.
--
-- States:
--   none    : default; user has never been a supporter.
--   active  : currently in good standing (one-time payment within
--             coverage window, or recurring subscription alive).
--   lapsed  : was active, but payment stopped or got cancelled.
--             Critically, lapsed users KEEP their pill + name_plate;
--             only the retention extension + accent revert to free-
--             tier. The "you supported once" recognition is permanent.

CREATE TABLE IF NOT EXISTS supporter_status (
    user_id              UUID        PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    state                TEXT        NOT NULL DEFAULT 'none',
    -- Optional 28-char display string shown on the supporter pill.
    -- Cap enforced at the API layer; left as plain TEXT here so a
    -- future cap change doesn't need a migration.
    name_plate           TEXT,
    became_supporter_at  TIMESTAMPTZ,
    last_payment_at      TIMESTAMPTZ,
    -- After a failed recurring payment, when does the user transition
    -- from `active` to `lapsed` if no further payment lands? NULL when
    -- not in a grace window.
    grace_until          TIMESTAMPTZ,
    cancelled_at         TIMESTAMPTZ,
    -- Revolut Business customer ID, populated on first successful
    -- checkout so subsequent recurring orders can be attached to the
    -- same Revolut customer record.
    revolut_customer_id  TEXT,
    updated_at           TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CHECK (state IN ('none', 'active', 'lapsed'))
);

CREATE INDEX IF NOT EXISTS supporter_status_state_idx
    ON supporter_status (state) WHERE state <> 'none';
