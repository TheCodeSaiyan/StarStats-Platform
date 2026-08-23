import type { BreakdownResponse } from '@/lib/api';
import type { TaxonomyLevel } from './TaxonomyStrip';

/**
 * Real dwell per place, from `/v1/me/location/breakdown`.
 *
 * A CORRECTION. Both `TaxonomyStrip` and `PlaceDetail` shipped ranked by VISIT
 * COUNT, each carrying a note saying the product had no per-place dwell and
 * that `Journey.jsx`'s `Dwell 171h` therefore could not be honoured. That was
 * wrong. `getLocationBreakdown` returns
 * `{ system, planet, city, dwell_seconds, visit_count }` per row — dwell,
 * aggregated at exactly the three levels the taxonomy strip ranks. It has been
 * in `lib/api.ts` the whole time; the travel page simply never called it, and
 * the local named `breakdownRes` there is the event-type metrics call, which
 * is what made it look as though the page already had this.
 *
 * `LocationFrequencyBars` was built against this endpoint and is still dead
 * code — nothing has rendered it since `/journey` became a redirect to `/me`.
 * `LocationHero` was in the same state and is now back on this page.
 *
 * THE SITE LEVEL IS NOT IN IT. The breakdown aggregates to city at its
 * deepest, so a site keeps the visit-count ranking. Callers must say which
 * measure they are showing rather than letting a reader assume the ranking is
 * the same all the way down — `dwellAvailableAt` answers that.
 */
export interface PlaceTotals {
  dwellSeconds: number;
  visits: number;
}

export type DwellIndex = Record<TaxonomyLevel, Map<string, PlaceTotals>>;

/** Levels the breakdown endpoint actually aggregates. */
export function dwellAvailableAt(level: TaxonomyLevel): boolean {
  return level !== 'site';
}

function fieldAt(
  e: BreakdownResponse['entries'][number],
  level: TaxonomyLevel,
): string | null | undefined {
  if (level === 'system') return e.system;
  if (level === 'planet') return e.planet;
  if (level === 'city') return e.city;
  return null;
}

export function buildDwellIndex(
  breakdown: BreakdownResponse | null,
): DwellIndex {
  const index: DwellIndex = {
    system: new Map(),
    planet: new Map(),
    city: new Map(),
    site: new Map(),
  };
  if (!breakdown?.entries) return index;

  for (const level of ['system', 'planet', 'city'] as const) {
    for (const e of breakdown.entries) {
      const name = fieldAt(e, level);
      if (!name) continue;
      const cur = index[level].get(name) ?? { dwellSeconds: 0, visits: 0 };
      // SUMMED, not assigned. The endpoint returns one row per
      // system/planet/city combination, so a system appears once per body
      // beneath it and a planet once per city — taking the last row would
      // report a fraction of the real total.
      cur.dwellSeconds += e.dwell_seconds;
      cur.visits += e.visit_count;
      index[level].set(name, cur);
    }
  }
  return index;
}

/** Total dwell across every place at a level — the denominator for a share. */
export function totalDwellAt(index: DwellIndex, level: TaxonomyLevel): number {
  let total = 0;
  for (const v of index[level].values()) total += v.dwellSeconds;
  return total;
}
