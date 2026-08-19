-- 0005_devices.sql -- Desktop client device pairing.
--
-- Two tables:
--
--  * `device_pairings` — short-lived pairing codes the user generates
--    on the website. The desktop client redeems one to receive a
--    long-lived device JWT. A pairing is single-use: `redeemed_at`
--    becomes non-NULL on first successful redemption.
--
--  * `devices` — persisted record of each paired desktop client.
--    Lets the user list and revoke devices from the website. The
--    JWT issued at redemption time carries the device row's id as a
--    custom claim; revocation is a `revoked_at` set + the verifier
--    rejects tokens whose device row is missing or revoked
--    (revocation enforcement lands in Slice 4 — for now, a deleted
--    pairing simply prevents *new* tokens being minted).
--
-- Pairing codes are 8 characters from a confusion-free alphabet
-- (no 0/O, no 1/I/l, no B/8). Stored in upper case to keep
-- comparisons case-insensitive without a functional index.

CREATE TABLE IF NOT EXISTS device_pairings (
    code         TEXT        PRIMARY KEY,
    user_id      UUID        NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    label        TEXT        NOT NULL,
    expires_at   TIMESTAMPTZ NOT NULL,
    redeemed_at  TIMESTAMPTZ,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS device_pairings_user_idx
    ON device_pairings (user_id, created_at DESC);

CREATE INDEX IF NOT EXISTS device_pairings_expiry_idx
    ON device_pairings (expires_at)
    WHERE redeemed_at IS NULL;

CREATE TABLE IF NOT EXISTS devices (
    id            UUID        PRIMARY KEY,
    user_id       UUID        NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    label         TEXT        NOT NULL,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_seen_at  TIMESTAMPTZ,
    revoked_at    TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS devices_user_active_idx
    ON devices (user_id, created_at DESC)
    WHERE revoked_at IS NULL;
