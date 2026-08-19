-- 0018_revolut_orders.sql -- Revolut Business order + webhook event tracking.
--
-- Lifecycle:
--   1. POST /v1/donate/checkout creates a `revolut_orders` row with our
--      generated `id` (UUIDv7). We pass that id as Revolut's
--      `merchant_order_ext_ref` so we can correlate webhook events.
--   2. We POST to the Merchant API. Revolut returns its own `id` and
--      `checkout_url`; we UPDATE the row with `revolut_order_id` +
--      `checkout_url` and return the URL to the client.
--   3. Customer pays on Revolut's hosted checkout page. Revolut
--      redirects them to our `redirect_url` (stop point — does NOT
--      mark anything as paid; only the webhook is trusted).
--   4. Revolut POSTs `ORDER_COMPLETED` to /v1/webhooks/revolut. Our
--      handler verifies the HMAC, then looks up the row by
--      `revolut_order_id`, marks it `completed`, and flips the user's
--      `supporter_status` to `active`.
--
-- The `revolut_webhook_events` table dedups redeliveries — Revolut
-- sometimes redelivers the same event, so we INSERT ON CONFLICT DO
-- NOTHING keyed by (revolut_order_id, event_type). A submission of
-- `ORDER_COMPLETED` for the same order twice will be no-op'd.

CREATE TABLE IF NOT EXISTS revolut_orders (
    id                      UUID        PRIMARY KEY,
    user_id                 UUID        NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    -- Populated on step 2 above. Nullable so the create-then-call
    -- sequence can land the row before Revolut returns.
    revolut_order_id        TEXT        UNIQUE,
    -- One of the keys in the application's TIERS array. Captured at
    -- checkout time so a future tier rename doesn't retroactively
    -- relabel historical donations.
    tier_key                TEXT        NOT NULL,
    amount_minor            BIGINT      NOT NULL,
    currency                TEXT        NOT NULL,
    -- The user's chosen 28-char display string at checkout time. We
    -- only persist it once payment lands — keeping the snapshot here
    -- means a user can change their plate later without rewriting
    -- history of past donations.
    name_plate_at_checkout  TEXT,
    state                   TEXT        NOT NULL DEFAULT 'pending',
    checkout_url            TEXT,
    created_at              TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    completed_at            TIMESTAMPTZ,
    CHECK (state IN ('pending', 'completed', 'cancelled', 'failed', 'refunded'))
);

CREATE INDEX IF NOT EXISTS revolut_orders_user_idx
    ON revolut_orders (user_id, created_at DESC);

CREATE TABLE IF NOT EXISTS revolut_webhook_events (
    revolut_order_id    TEXT        NOT NULL,
    event_type          TEXT        NOT NULL,
    received_at         TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    -- Whole webhook payload. We index on the dedup PK only; the JSONB
    -- is forensic state ("what did Revolut tell us when") rather than
    -- a hot read path.
    payload             JSONB       NOT NULL,
    PRIMARY KEY (revolut_order_id, event_type)
);
