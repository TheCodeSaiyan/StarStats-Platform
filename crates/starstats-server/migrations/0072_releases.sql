-- Release notes, one row per release tag.
--
-- Written by CI (POST /v1/internal/releases, HMAC-signed) after a release
-- is tagged, from scripts/release-notes.mjs: player-facing notes built from
-- commit subjects and roadmap items, grouped New / Improved / Fixed. Read by
-- the tray's What's New, the web's /whats-new and /changelog.
--
-- `notes` holds the groups exactly as the generator emits them
-- ([{kind, lines: [{text, surfaces, prs, roadmap}]}]); nothing queries
-- inside it. `tag` is unique so a re-run of the same release replaces its
-- row instead of adding a second.
--
-- One-way: additive only; an older server never reads these tables.

CREATE TABLE IF NOT EXISTS releases (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    track       TEXT NOT NULL,
    tag         TEXT NOT NULL,
    version     TEXT NOT NULL,
    channel     TEXT NOT NULL,
    released_on DATE NOT NULL,
    summary     TEXT NOT NULL DEFAULT '',
    notes       JSONB NOT NULL DEFAULT '[]'::jsonb,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE UNIQUE INDEX IF NOT EXISTS releases_tag_idx ON releases (tag);

-- Feeds: newest first per track, optionally limited to channels.
CREATE INDEX IF NOT EXISTS releases_track_idx ON releases (track, released_on DESC, created_at DESC);

CREATE TABLE IF NOT EXISTS release_reads (
    user_id    UUID NOT NULL,
    release_id UUID NOT NULL,
    seen_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (user_id, release_id)
);
