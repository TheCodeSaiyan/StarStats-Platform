-- Local SQLite schema for the StarStats tray client.
-- Append-only events + tail offset cursor.

CREATE TABLE IF NOT EXISTS events (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    idempotency_key TEXT    NOT NULL UNIQUE,
    type            TEXT    NOT NULL,
    timestamp       TEXT    NOT NULL,
    raw             TEXT    NOT NULL,
    payload         TEXT    NOT NULL,
    log_source      TEXT    NOT NULL DEFAULT 'live',
    source_offset   INTEGER NOT NULL DEFAULT 0,
    inserted_at     TEXT    NOT NULL DEFAULT (datetime('now')),
    -- Optional EventMetadata blob stamped at enrichment time. Held as
    -- JSON because SQLite has no native struct type and the metadata is
    -- only ever read/written wholesale (no per-field SQL queries). NULL
    -- means no enrichment has happened for this row yet; that's the
    -- vast-majority case so the storage cost is per-enriched-row only.
    metadata        TEXT,
    -- Timestamp at which this row was successfully POSTed to the
    -- StarStats API (and the server returned a 2xx). NULL = still in
    -- the local outbox. Replaces the global `sync_cursor` model: with
    -- priority lanes the urgent worker drains specific event_types out
    -- of monotonic id order, so a single high-water-mark cursor can't
    -- represent the drain state correctly. Per-row flag lets either
    -- lane pick its next batch with a simple `WHERE sent_at IS NULL
    -- [AND type IN (...)]` filter. Existing rows are backfilled at
    -- migration time from the legacy cursor — see
    -- `Storage::migrate_events_sent_at`.
    sent_at         TEXT
);
CREATE INDEX IF NOT EXISTS idx_events_type_ts ON events(type, timestamp);
CREATE INDEX IF NOT EXISTS idx_events_inserted ON events(inserted_at);
-- The partial index on `events(id) WHERE sent_at IS NULL` is created
-- by `Storage::migrate_events_sent_at` instead of here. Reason: this
-- file runs BEFORE the column-add migrations, and SQLite rejects an
-- index whose WHERE clause references a column that doesn't exist
-- yet. Keeping the index colocated with the migration also means a
-- legacy DB that's missing both column and index reaches a
-- consistent state in one place.

CREATE TABLE IF NOT EXISTS tail_cursor (
    path        TEXT PRIMARY KEY,
    offset      INTEGER NOT NULL DEFAULT 0,
    -- Signature of the physical file the offset belongs to (see
    -- `file_signature` in gamelog.rs). NULL for the generic
    -- id-high-water-mark rows written by org_connector, and for
    -- legacy rows created before this column existed. A changed
    -- signature means the file was rotated/replaced → resume at head.
    file_sig    TEXT,
    updated_at  TEXT NOT NULL DEFAULT (datetime('now'))
);

-- Cursor for "what's been shipped to the API server" — used by the
-- sync worker (Phase 1b). Distinct from tail_cursor (which tracks
-- "what's been read from disk").
CREATE TABLE IF NOT EXISTS sync_cursor (
    last_event_id INTEGER NOT NULL,
    updated_at    TEXT    NOT NULL DEFAULT (datetime('now'))
);

-- Lines that the structural parser recognised (gave us a stable
-- timestamp + event_name) but for which the classifier had no rule.
-- Dedupe by (log_source, event_name); we keep the most recent body
-- as a sample, plus first_seen / last_seen for forensic context, and
-- bump occurrences so the user can see which unknowns are common.
--
-- This table is the input for two later features:
--   1. UI surface — "you have N unknown event types, here they are"
--   2. Crowd-sourced rules — `sample_body` is what a user-submitted
--      regex would actually be tested against.
--
-- No-shell lines (banners, blanks, continuation lines) are NOT
-- recorded here — they're not actionable as parser rules.
CREATE TABLE IF NOT EXISTS unknown_event_samples (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    log_source   TEXT    NOT NULL,
    event_name   TEXT    NOT NULL,
    occurrences  INTEGER NOT NULL DEFAULT 1,
    first_seen   TEXT    NOT NULL DEFAULT (datetime('now')),
    last_seen    TEXT    NOT NULL DEFAULT (datetime('now')),
    sample_line  TEXT    NOT NULL,
    sample_body  TEXT    NOT NULL,
    UNIQUE(log_source, event_name)
);
CREATE INDEX IF NOT EXISTS idx_unknown_samples_occurrences
    ON unknown_event_samples (occurrences DESC, last_seen DESC);

-- Event names we deliberately ignore. Peer concept to "rules" — when
-- the rules engine ships, this is how users say "this is engine-
-- internal noise, never show it in the unknowns list, never propose a
-- rule for it."
--
-- `source` is informational: 'builtin' for the seeded defaults the
-- app ships, 'user' for entries added via the tray UI, 'community'
-- (future) for entries pulled from the central rules service.
--
-- The unique key is (event_name) — we don't currently scope by
-- log_source because engine internals fire identically on LIVE/PTU/
-- EPTU. If that ever stops being true, add log_source to the PK.
CREATE TABLE IF NOT EXISTS event_noise_list (
    event_name  TEXT    NOT NULL PRIMARY KEY,
    source      TEXT    NOT NULL DEFAULT 'user',
    added_at    TEXT    NOT NULL DEFAULT (datetime('now'))
);

-- Cached parser-definition manifest fetched from the server's
-- `GET /v1/parser-definitions`. Only one row at a time — `id = 1` is
-- a sentinel so an UPSERT replaces the cache instead of accumulating
-- stale rows. `payload_json` holds the full Manifest as a JSON
-- blob; the client deserialises + compiles on startup or after a
-- successful fetch.
CREATE TABLE IF NOT EXISTS parser_def_manifest (
    id            INTEGER PRIMARY KEY CHECK (id = 1),
    version       INTEGER NOT NULL,
    fetched_at    TEXT    NOT NULL DEFAULT (datetime('now')),
    payload_json  TEXT    NOT NULL
);

-- Per-device monotonic ingest-batch counter (F7 `batch_sequence`). One
-- row at a time — `id = 1` sentinel, same shape as parser_def_manifest.
-- `value` is the highest batch ordinal this install has SUCCESSFULLY
-- sent (2xx). The wire value for the next send is `value + 1`; the
-- counter only advances on success (`commit_batch_sequence`, a
-- `MAX(value, excluded.value)` upsert), so retried/poison-bisected sends
-- reuse their number instead of burning it — no false server-side gaps.
-- Absent row (fresh install) reads as 0 → first batch ships sequence 1.
CREATE TABLE IF NOT EXISTS batch_sequence_counter (
    id     INTEGER PRIMARY KEY CHECK (id = 1),
    value  INTEGER NOT NULL DEFAULT 0
);

-- Phase 4.B local cache of `UnknownLine` records produced by
-- `classify_or_capture`. Dedupe key is `shape_hash` — collapsing
-- value-varying lines (timestamps, GEIDs, UUIDs, etc.) onto one row so
-- a chatty unknown doesn't spam the review queue. `raw_examples_json`
-- stores up to 5 raw line samples (newest at the end, oldest dropped
-- when a sixth arrives) so a reviewer can compare the original strings
-- against the shape template. PII detection results, partial parsed
-- fields, and surrounding context are all JSON blobs because they're
-- read whole on the review screen and never queried inside SQL.
--
-- `dismissed = 1` hides the row from `list_unknown_lines` without
-- deleting it (so a future re-open doesn't re-surface the same
-- pattern). `submitted_at` is set by `mark_submitted` once the user
-- ships the row to the moderation queue; presence of a timestamp also
-- implies the row should stop accruing more raw samples.
CREATE TABLE IF NOT EXISTS unknown_lines (
    id                       TEXT    PRIMARY KEY,
    shape_hash               TEXT    NOT NULL UNIQUE,
    raw_examples_json        TEXT    NOT NULL,
    partial_structured_json  TEXT    NOT NULL,
    shell_tag                TEXT,
    context_before_json      TEXT    NOT NULL,
    context_after_json       TEXT    NOT NULL,
    game_build               TEXT,
    channel                  TEXT    NOT NULL,
    interest_score           INTEGER NOT NULL,
    occurrence_count         INTEGER NOT NULL DEFAULT 1,
    first_seen               TEXT    NOT NULL,
    last_seen                TEXT    NOT NULL,
    detected_pii_json        TEXT    NOT NULL,
    dismissed                INTEGER NOT NULL DEFAULT 0,
    submitted_at             TEXT
);
CREATE INDEX IF NOT EXISTS unknown_lines_interest
    ON unknown_lines(dismissed, interest_score DESC, occurrence_count DESC);
CREATE INDEX IF NOT EXISTS unknown_lines_shape
    ON unknown_lines(shape_hash);
