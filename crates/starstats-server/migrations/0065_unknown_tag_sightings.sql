-- ============================================================================
-- 0065_unknown_tag_sightings.sql — parser-health tag correlation (additive).
--
-- Spec 1 (migration 0064) detects that an event type has gone dark. It cannot
-- say WHY, because the replacement log tag only exists client-side in the
-- tray's `unknown_lines` queue and nothing about unknown lines crosses the
-- wire. This adds an opt-in, metadata-only channel so the detector can name
-- the tag that appeared when a type died.
--
-- Privacy: shell tags ONLY — engine symbol names like
-- `LandingArea_UnregisterFromExternalSystems_StowingVehicle`. No raw log line
-- bodies, ever. The tray validates each tag against a conservative charset
-- before send, and the server re-validates on receipt. Off by default; the
-- user opts in per-tray.
--
-- Also backfills the collapse timestamp onto findings. `first_flagged_at` is
-- when the detector NOTICED (up to a week late); `last_event_at` is when the
-- type actually stopped, which is what a candidate tag's first sighting must
-- be correlated against.
-- ============================================================================

ALTER TABLE parser_health_finding
    ADD COLUMN IF NOT EXISTS last_event_at TIMESTAMPTZ;

CREATE TABLE IF NOT EXISTS unknown_tag_sighting (
    claimed_handle TEXT        NOT NULL,
    -- The `<EventName>` shell tag of a log line the parser could not classify.
    shell_tag      TEXT        NOT NULL,
    first_seen     TIMESTAMPTZ NOT NULL,
    last_seen      TIMESTAMPTZ NOT NULL,
    occurrences    BIGINT      NOT NULL DEFAULT 0,
    -- Game build string when known; lets a correlation be tied to a patch.
    game_build     TEXT,
    updated_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (claimed_handle, shell_tag),
    CONSTRAINT unknown_tag_sighting_lc_chk CHECK (claimed_handle = lower(claimed_handle))
);

-- Correlation reads by first_seen window, so index that directly.
CREATE INDEX IF NOT EXISTS unknown_tag_sighting_first_seen_idx
    ON unknown_tag_sighting (first_seen DESC);
