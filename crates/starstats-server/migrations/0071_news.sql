-- News posts: announcements written by staff in the admin console and
-- shown in the tray's What's New and on the web.
--
-- Deliberately not notification rows. A post is written once and read by
-- everyone, so it lives here once; per-player state is only "have I seen
-- it", in news_reads, written when the player opens it.
--
-- `body` is plain text. It is rendered with whitespace preserved and never
-- as HTML, so a post cannot carry markup into either client.
--
-- `published_at` NULL = draft. `deleted_at` is a soft delete so the audit
-- trail and read state keep pointing at something.
--
-- One-way: additive only; an older server simply never reads these.

CREATE TABLE IF NOT EXISTS news_posts (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    title        TEXT NOT NULL,
    body         TEXT NOT NULL,
    link_url     TEXT NULL,
    created_by   TEXT NOT NULL,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    published_at TIMESTAMPTZ NULL,
    deleted_at   TIMESTAMPTZ NULL
);

-- The reader feed: live posts, newest first.
CREATE INDEX IF NOT EXISTS news_posts_published_idx
    ON news_posts (published_at DESC)
    WHERE published_at IS NOT NULL AND deleted_at IS NULL;

CREATE TABLE IF NOT EXISTS news_reads (
    user_id  UUID NOT NULL,
    news_id  UUID NOT NULL,
    seen_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (user_id, news_id)
);
