-- 0003_event_seq.sql -- Add a monotonic cursor column for pagination.
--
-- The primary key (UUID v7) is already time-ordered, but a BIGSERIAL
-- gives us a friendlier integer cursor for "after_id" pagination and
-- avoids exposing UUIDs in client-facing APIs.

ALTER TABLE events
    ADD COLUMN IF NOT EXISTS seq BIGSERIAL;

-- Existing rows get backfilled by BIGSERIAL automatically. Add the
-- index AFTER the column population so building it is cheap.
CREATE UNIQUE INDEX IF NOT EXISTS events_seq_uq ON events (seq);

-- Pagination index: (claimed_handle, seq) for "give me my next page".
CREATE INDEX IF NOT EXISTS events_handle_seq_idx
    ON events (claimed_handle, seq DESC);
