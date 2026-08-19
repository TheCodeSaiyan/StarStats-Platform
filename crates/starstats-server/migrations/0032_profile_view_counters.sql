-- Per-day, per-source view counters on the public profile.
--
-- Piece 2 of the public-profile UX work. We count views into the
-- public profile so the owner can see how their share is performing.
-- The primary key (profile_handle, day, source) folds same-source same-
-- day hits into a single row via UPSERT — keeps the table small even
-- for a heavy referrer (≤ 4 sources * 1 row per active day per handle).
--
-- Migrations are additive only: IF NOT EXISTS guards the table + both
-- indexes so a re-run on an environment that's already on this version
-- is a no-op. The two indexes mirror the two read patterns:
--   - read_stats(handle, days)  → (handle, day DESC)
--   - admin / analytics scans   → (day DESC)
CREATE TABLE IF NOT EXISTS public_profile_view_counters (
    profile_handle TEXT NOT NULL,
    day DATE NOT NULL,
    source TEXT NOT NULL,
    view_count INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (profile_handle, day, source)
);

CREATE INDEX IF NOT EXISTS public_profile_view_counters_day
    ON public_profile_view_counters(day DESC);

CREATE INDEX IF NOT EXISTS public_profile_view_counters_handle_day
    ON public_profile_view_counters(profile_handle, day DESC);
