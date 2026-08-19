-- 0023_share_metadata.sql — Per-user share metadata (expiry + notes).
--
-- SpiceDB stores the share relationship itself (one
-- `stats_record:<owner>#share_with_user@user:<recipient>` row per
-- grant). It has no concept of time, so we keep all temporal /
-- annotation data here instead. The metadata is purely *advisory*
-- when read; enforcement is the read-time check in the friend_*
-- handlers, which 404 + lazily delete the SpiceDB row when
-- `expires_at < now()`.
--
-- Case-insensitive uniqueness is enforced by a UNIQUE INDEX over
-- `lower(...)` expressions rather than the table's PRIMARY KEY
-- clause: Postgres does not accept expressions inside
-- `PRIMARY KEY (...)` (parse error "syntax error at or near '('"
-- caught on the homelab roll-out). ON CONFLICT inside the upsert
-- handler already targets the expression form, which Postgres
-- resolves against a matching unique index — so the upsert path is
-- unchanged. RSI handles are case-preserved on display but
-- case-insensitive for lookup; SpiceDB ids inherit that.
--
-- No FK to `users`: a metadata row can outlive a deleted account
-- (the SpiceDB cleanup happens separately), and we don't want a
-- user-deletion path to take this table down with it.

CREATE TABLE IF NOT EXISTS share_metadata (
    owner_handle     TEXT        NOT NULL,
    recipient_handle TEXT        NOT NULL,
    -- NULL = share never expires (matches the previous wave's
    -- behaviour, which had no expiry concept at all).
    expires_at       TIMESTAMPTZ NULL,
    -- Free-text human note. Capped on write (validation in the
    -- handler) to keep the column bounded.
    note             TEXT        NULL,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Effective primary key — case-insensitive uniqueness over the
-- handle pair. ON CONFLICT (lower(owner_handle), lower(recipient_handle))
-- in the upserter targets this index.
CREATE UNIQUE INDEX IF NOT EXISTS share_metadata_owner_recipient_lower_uniq
    ON share_metadata (lower(owner_handle), lower(recipient_handle));

-- Forward-looking: a future background sweep can batch-revoke
-- expired rows via `WHERE expires_at < now()`. Partial index keeps
-- the never-expires rows out of the index entirely.
CREATE INDEX IF NOT EXISTS share_metadata_expires_at_idx
    ON share_metadata (expires_at)
    WHERE expires_at IS NOT NULL;

-- Reverse lookup for the inbound side (list_shared_with_me + the
-- friend read endpoints both filter by recipient).
CREATE INDEX IF NOT EXISTS share_metadata_recipient_lower_idx
    ON share_metadata (lower(recipient_handle));
