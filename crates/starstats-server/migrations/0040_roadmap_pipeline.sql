-- 0040_roadmap_pipeline.sql -- Roadmap pipeline (spec
-- `docs/ROADMAP-PIPELINE-SPEC.md`).
--
-- Phase 1 of the build plan: lays down every table the roadmap
-- pipeline reads from. No routes are wired yet -- those come in later
-- phases. Migrations follow the project posture: additive only, every
-- statement guarded by `IF NOT EXISTS`, byte-immutable post-deploy.
--
-- Closed-vocabulary enums (`status`, `build_health`, `eta_band`,
-- `category`, `channel`, link `kind`) are stored as plain TEXT and
-- validated in Rust via `parse()` + `as_str()`. Adding a variant does
-- not require a migration. See `src/roadmap/models.rs` for the
-- authoritative variant list.
--
-- No FK constraints to `users` / `organizations` / etc.: the project
-- treats referential integrity as an application-layer concern (see
-- `share_metadata.rs` "No FK to users"). Cascading deletes are not
-- relied on; soft-deletes are explicit.
--
-- No GRANT statements: StarStats runs a single Postgres role and
-- permissions are managed outside the SQLx migration tree.

-- ----------------------------------------------------------------
-- roadmap_items
-- ----------------------------------------------------------------
-- One row per Project item on the GitHub roadmap Project board. The
-- slug is generated on first sync from the item title and is
-- immutable thereafter (§1.4); `github_project_item_id` is the stable
-- handle across renames.
CREATE TABLE IF NOT EXISTS roadmap_items (
    id                       UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    slug                     TEXT        NOT NULL UNIQUE,
    github_project_item_id   TEXT        NOT NULL UNIQUE,
    title                    TEXT        NOT NULL,
    summary                  TEXT        NULL,
    category                 TEXT        NULL,
    eta_band                 TEXT        NULL,
    -- Denormalised vote count. Local row is authoritative (§1.3);
    -- writeback to GitHub is a denormalised mirror.
    votes                    INTEGER     NOT NULL DEFAULT 0,
    -- Surfaces the item appears on (e.g. tray-whats-new, web-roadmap,
    -- in-context-tooltip, admin-only). Closed vocabulary, validated
    -- application-side.
    surfaces                 TEXT[]      NOT NULL DEFAULT ARRAY[]::TEXT[],
    parent_id                UUID        NULL,
    -- Typed array of {kind, url, label}. Closed vocabulary on `kind`,
    -- validated at the API layer (§1.7).
    links                    JSONB       NOT NULL DEFAULT '[]'::jsonb,
    public                   BOOLEAN     NOT NULL DEFAULT FALSE,
    -- Split last_updated timestamps (§1.8). Content vs pipeline edits
    -- bump independently; vote writeback bumps neither.
    content_last_updated     TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    pipeline_last_updated    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    created_at               TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    -- Soft-delete column. Project item deletion on GitHub flips this;
    -- soft-deleted rows never appear on any surface (§3.4).
    deleted_at               TIMESTAMPTZ NULL
);

-- Listing filter: most public surfaces filter on
-- (deleted_at IS NULL AND public = TRUE). Partial index keeps the
-- hot path tight.
CREATE INDEX IF NOT EXISTS roadmap_items_public_live_idx
    ON roadmap_items (created_at DESC)
    WHERE deleted_at IS NULL AND public = TRUE;

-- Admin/dev view filter: all non-deleted items.
CREATE INDEX IF NOT EXISTS roadmap_items_live_idx
    ON roadmap_items (created_at DESC)
    WHERE deleted_at IS NULL;

-- Parent lookup for component roll-ups (§2.7).
CREATE INDEX IF NOT EXISTS roadmap_items_parent_idx
    ON roadmap_items (parent_id)
    WHERE parent_id IS NOT NULL;

-- ----------------------------------------------------------------
-- roadmap_channel_statuses
-- ----------------------------------------------------------------
-- Per-(item, channel) status row (§2.1). A channel only appears here
-- when it is either listed on the Project's `Channels` field or a CI
-- event has been received for it. Removed channels are moved to
-- `roadmap_channel_statuses_archive` (not hard-deleted) so vote
-- history and `previous_shipped_sha` are recoverable on re-add.
CREATE TABLE IF NOT EXISTS roadmap_channel_statuses (
    roadmap_item_id      UUID        NOT NULL,
    channel              TEXT        NOT NULL,
    status               TEXT        NOT NULL DEFAULT 'proposed',
    build_health         TEXT        NOT NULL DEFAULT 'unknown',
    build_id             TEXT        NULL,
    commit_sha           TEXT        NULL,
    deployed_at          TIMESTAMPTZ NULL,
    ci_run_url           TEXT        NULL,
    previous_shipped_sha TEXT        NULL,
    last_event_id        TEXT        NULL,
    updated_at           TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (roadmap_item_id, channel)
);

-- Reverse lookups: status-by-channel (e.g. "all items shipped on live").
CREATE INDEX IF NOT EXISTS roadmap_channel_statuses_channel_status_idx
    ON roadmap_channel_statuses (channel, status);

-- ----------------------------------------------------------------
-- roadmap_channel_statuses_archive
-- ----------------------------------------------------------------
-- Holding pen for channel-status rows that were removed from a
-- roadmap item (channel dropped from `Channels` field). Re-adding
-- the channel restores from this table.
CREATE TABLE IF NOT EXISTS roadmap_channel_statuses_archive (
    roadmap_item_id      UUID        NOT NULL,
    channel              TEXT        NOT NULL,
    status               TEXT        NOT NULL,
    build_health         TEXT        NOT NULL,
    build_id             TEXT        NULL,
    commit_sha           TEXT        NULL,
    deployed_at          TIMESTAMPTZ NULL,
    ci_run_url           TEXT        NULL,
    previous_shipped_sha TEXT        NULL,
    last_event_id        TEXT        NULL,
    archived_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (roadmap_item_id, channel)
);

-- ----------------------------------------------------------------
-- roadmap_event_log
-- ----------------------------------------------------------------
-- Idempotency log for inbound CI events (§4.4). A second event with
-- the same `event_id` is dropped without state change. Rows are
-- pruned in code after 14 days; no DB-level TTL.
CREATE TABLE IF NOT EXISTS roadmap_event_log (
    event_id    TEXT        PRIMARY KEY,
    received_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- TTL sweep index: the 14-day prune loop ranges over received_at.
CREATE INDEX IF NOT EXISTS roadmap_event_log_received_at_idx
    ON roadmap_event_log (received_at);

-- ----------------------------------------------------------------
-- roadmap_votes
-- ----------------------------------------------------------------
-- One row per (user, roadmap_item). Local DB is authoritative
-- (§1.3); GitHub `Votes` field is a denormalised mirror.
CREATE TABLE IF NOT EXISTS roadmap_votes (
    user_id         UUID        NOT NULL,
    roadmap_item_id UUID        NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (user_id, roadmap_item_id)
);

-- Aggregate `count(*) GROUP BY roadmap_item_id` for vote-recompute on
-- retract / insert and for the 5-min writeback batcher.
CREATE INDEX IF NOT EXISTS roadmap_votes_item_idx
    ON roadmap_votes (roadmap_item_id);

-- "What did this user vote for?" -- powers the personal dashboard
-- and rate-limit window.
CREATE INDEX IF NOT EXISTS roadmap_votes_user_created_idx
    ON roadmap_votes (user_id, created_at DESC);

-- ----------------------------------------------------------------
-- roadmap_subscribers
-- ----------------------------------------------------------------
-- One row per (user, roadmap_item). Subscriber membership is
-- sensitive (interest inference -- §7.2) and is never returned
-- publicly; only the aggregate count is.
CREATE TABLE IF NOT EXISTS roadmap_subscribers (
    user_id         UUID        NOT NULL,
    roadmap_item_id UUID        NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (user_id, roadmap_item_id)
);

-- Notification fan-out path: "who is subscribed to this item".
CREATE INDEX IF NOT EXISTS roadmap_subscribers_item_idx
    ON roadmap_subscribers (roadmap_item_id);

-- "What does this user follow" -- personal dashboard.
CREATE INDEX IF NOT EXISTS roadmap_subscribers_user_idx
    ON roadmap_subscribers (user_id);

-- ----------------------------------------------------------------
-- roadmap_changelog
-- ----------------------------------------------------------------
-- Auto-drafted changelog entries (§8). Entries are drafted when a
-- channel flips to `shipped` and stay as drafts until an admin
-- publishes them. Unpublished drafts auto-purge at 30 days.
CREATE TABLE IF NOT EXISTS roadmap_changelog (
    id              UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    roadmap_item_id UUID        NOT NULL,
    channel         TEXT        NOT NULL,
    title           TEXT        NOT NULL,
    body            TEXT        NOT NULL,
    -- Stable shipped-SHA pair the entry was drafted from.
    previous_shipped_sha TEXT   NULL,
    shipped_sha     TEXT        NULL,
    -- Draft -> published transition. NULL `published_at` is the draft
    -- state.
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    published_at    TIMESTAMPTZ NULL,
    published_by    TEXT        NULL
);

-- Publish-state queue index: admin draft view + public changelog
-- view both filter on published_at.
CREATE INDEX IF NOT EXISTS roadmap_changelog_published_idx
    ON roadmap_changelog (published_at DESC NULLS LAST, created_at DESC);

-- Per-item history: detail page surfaces this item's changelog.
CREATE INDEX IF NOT EXISTS roadmap_changelog_item_idx
    ON roadmap_changelog (roadmap_item_id, created_at DESC);

-- Auto-purge sweep: drafts older than 30 days. Partial index keeps
-- published rows out of the prune scan.
CREATE INDEX IF NOT EXISTS roadmap_changelog_draft_purge_idx
    ON roadmap_changelog (created_at)
    WHERE published_at IS NULL;

-- ----------------------------------------------------------------
-- roadmap_user_read_state
-- ----------------------------------------------------------------
-- Per-user "unread" tracking for the tray "What's new" panel (§9).
-- Server-side so state syncs across devices.
CREATE TABLE IF NOT EXISTS roadmap_user_read_state (
    user_id                       UUID        NOT NULL,
    roadmap_item_id               UUID        NOT NULL,
    last_seen_changelog_entry_id  UUID        NULL,
    last_seen_at                  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (user_id, roadmap_item_id)
);

-- "Unread for this user" path; partner of the changelog publish
-- query.
CREATE INDEX IF NOT EXISTS roadmap_user_read_state_user_idx
    ON roadmap_user_read_state (user_id, last_seen_at DESC);
