-- ============================================================================
-- 0068_events_planet_idx.sql — partial expression index so "planets visited"
-- can be a REAL distinct count instead of a capped bucket list.
--
-- WHAT IT REPLACES. `stats_travel` returned `planets_visited` as a
-- `payload_field_breakdown` bounded by STATS_BUCKET_LIMIT (100), and the web
-- rendered `planets_visited.length`. A list length standing in for a count:
-- past 100 distinct planets it pins and stops reporting, silently.
--
-- WHY AN INDEX RATHER THAN A BETTER QUERY. Measured against 30,000
-- `planet_terrain_load` rows holding 15 distinct planets — the same shape as
-- production, where a 440k-event handle has 30,701 such rows and 15 planets:
--
--     count(DISTINCT payload->>'planet')          16.0 ms   seq scan + quicksort 2.6MB
--     count(*) FROM (SELECT DISTINCT …)            8.1 ms   seq scan + HashAggregate
--     loose index scan, WITH this index            0.3 ms   16 index descents, no heap
--
-- The recursive/loose form is only fast BECAUSE of this index; without it each
-- descent degrades into a scan and it is worse than either alternative. The
-- two travel together.
--
-- The shape is what matters, not the multiple: cost tracks the number of
-- distinct planets (a property of the game, ~15) rather than the number of
-- events (a property of how long someone has played, and unbounded). A
-- history of millions of events costs the same as one of thousands.
--
-- PARTIAL on purpose. Only `planet_terrain_load` rows are indexed — roughly 7%
-- of the events table on the handle measured — so this stays small and does
-- not weigh on ingest for every other event type.
--
-- Additive per the migration contract: CREATE INDEX IF NOT EXISTS, no DROP,
-- no table rewrite. Building it takes a moment on a large table but does not
-- block reads.
-- ============================================================================

CREATE INDEX IF NOT EXISTS events_handle_planet_idx
    ON events (claimed_handle, (payload ->> 'planet'))
    WHERE event_type = 'planet_terrain_load';
