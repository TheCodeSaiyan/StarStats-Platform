import React from 'react';
import type { Route } from 'next';
import { getRoutes } from '@/lib/api';
import type { RoutesResponse } from '@/lib/api';
import { loadAllReferenceBundles, type ReferenceCatalog } from '@/lib/reference';
import { EntityLink } from '@/components/kb/EntityLink';
import { aggregateLocationBuckets } from '@/lib/class-name-parts';
import { logger } from '@/lib/logger';
import { rangeToWindowHours, rangeHasLifetimeBaseline } from '@/lib/range';
import { computeTrend, formatTrend, previousWindowLabel } from '@/lib/trend';
import { EmptyWindow } from './kit/EmptyWindow';
import { defineWidget } from './kit/defineWidget';
import { RankedList } from './kit/archetypes';
import { fmtNum } from './kit/format';

/**
 * `routes` — "Top routes": most-travelled quantum destinations from the
 * reparse-gated `GET /v1/me/stats/routes` aggregate (`quantum_route`
 * events), ranked by trip count. Owner-only (me-scoped), range-aware
 * (follows the dashboard range selector).
 *
 * Migrated to the kit: `RankedList` owns the cap + "See all" link so the
 * tile never scrolls (the pre-migration body sliced to a fixed 8 and let
 * the tile scroll — the exact overflow bug this fixes). Destinations are
 * location identifiers, each wrapped in `<EntityLink category="location">`
 * with `label` pinned so a free-text destination is never rewritten by the
 * class-id prettifier (docs/ENGINEERING.md free-text rule).
 */
interface RoutesData {
  routes: ReadonlyArray<{ destination: string; count: number }>;
  locations: ReferenceCatalog;
  /** Lifetime baseline for the windowed list (UX Rule 2). Null on the
   *  `all` range, which spans the whole of retention — its twin covers
   *  the same rows, so the note would only restate the list. */
  lifetime: NonNullable<RoutesResponse['lifetime']> | null;
  /** Same-length window immediately before this one. Null means "no
   *  comparison to draw" -- the server omits it when the handle had no
   *  activity at all back then (not a user yet) and on `all`, which has
   *  no predecessor inside retention. NEVER coerce to 0: a real zero
   *  means "played, but none of this", which reads very differently. */
  previous: NonNullable<RoutesResponse['previous']> | null;
}

/** Provenance caveat — quantum_route events, not every hop travelled. */
const QT_CAVEAT = 'Based on quantum travel';

export const routesWidget = defineWidget<RoutesData>({
  id: 'routes',
  eyebrow: 'Routes',
  rangeAware: true,
  visibility: 'owner',
  async load(ctx) {
    if (!ctx.token) return null;
    let routes: ReadonlyArray<{ destination: string; count: number }> = [];
    let lifetime: RoutesData['lifetime'] = null;
    let previous: RoutesData['previous'] = null;
    try {
      const res = await getRoutes(ctx.token, rangeToWindowHours(ctx.range));
      routes = res?.routes ?? [];
      // Dropped on `all`: that range already spans retention, so the
      // twin covers the same rows and the note would read "N of N".
      lifetime = rangeHasLifetimeBaseline(ctx.range)
        ? (res?.lifetime ?? null)
        : null;
      // Not gated on the range here: the SERVER already omits `previous`
      // for `all` (no predecessor inside retention) and for a handle with
      // no prior activity. Re-deciding it client-side would just risk the
      // two rules drifting apart.
      previous = res?.previous ?? null;
    } catch (err) {
      logger.warn({ err, call: 'widget.routes' }, 'fetch failed');
      return null;
    }
    // An empty WINDOW is not the same as having no routes. These widgets
    // were lifetime-only until #309 range-scoped them; since then a
    // handle whose quantum travel predates the selected range gets an
    // empty list and the tile goes blank — indistinguishable from "this
    // feature is broken", which is exactly how it was reported.
    //
    // The lifetime twin knows the difference, so say it. Only bail
    // completely when there is nothing to report either way (which
    // includes the `all` range, where `lifetime` is deliberately null
    // because the window already spans retention).
    if (routes.length === 0 && !(lifetime && lifetime.total_trips > 0)) {
      return null;
    }
    const { catalogs } = await loadAllReferenceBundles();
    return { routes, locations: catalogs.locations, lifetime, previous };
  },
  body(data, ctx) {
    // Empty window, but the handle HAS routes outside it. Name both
    // figures and point at the fix, rather than rendering the same
    // blank tile a genuinely empty account would get.
    if (data.routes.length === 0) {
      return (
        <EmptyWindow
          rangeLabel={previousWindowLabel(ctx.range)}
          lifetimeCount={data.lifetime?.total_trips ?? 0}
          noun="quantum routes"
        />
      );
    }
    // Resolve raw engine ids (LOC_RR_*, MISSION_QT_* beacons, pipe
    // hierarchies) to friendly labels and merge duplicates (e.g. all
    // per-mission beacons → one "Mission beacon" row).
    const agg = aggregateLocationBuckets(
      data.routes.map((r) => ({ value: r.destination, count: r.count })),
    );
    const rows = agg.map((a) => ({
      key: a.label,
      // Deep-link each row to the KB using the FRIENDLY label as the
      // classKey — the catalog is dual-keyed by `display_name`, so real
      // places ("microTech", "New Babbage") resolve and link, while
      // synthetic labels ("Mission beacon", "Rest Stop S1 L1") miss and
      // EntityLink degrades to plain text. `label` is pinned so the
      // class-id prettifier never rewrites the friendly destination.
      label: (
        <EntityLink
          category="location"
          classKey={a.label}
          catalog={data.locations}
          label={a.label}
        />
      ),
      value: fmtNum(a.count),
    }));
    // Trips this window against the lifetime trip count. Aggregation merges
    // destinations but preserves counts, so the window sum is like-for-like
    // with `total_trips`. `lifetime.destinations` is deliberately NOT shown:
    // it counts RAW destinations, while the rows above are merged buckets —
    // "4 of 22 places" beside 3 rows would be a comparison of two different
    // quantities. Only rendered when the server sent the baseline.
    const lifetime = data.lifetime;
    const windowTrips = data.routes.reduce((acc, r) => acc + r.count, 0);
    const compare = lifetime
      ? `${fmtNum(windowTrips)} of ${fmtNum(lifetime.total_trips)} trips all time`
      : null;
    // Trend on TRIPS only, for the same reason `destinations` is not
    // shown above: it is a distinct count over merged buckets, so a
    // period delta on it compares two different quantities.
    const trend = data.previous
      ? formatTrend(
          computeTrend(windowTrips, data.previous.total_trips),
          previousWindowLabel(ctx.range),
          fmtNum,
        )
      : null;
    // The quantum-travel caveat stays: it bounds what these rows are drawn
    // from, which the comparison doesn't say.
    const note = [trend ?? compare, QT_CAVEAT].filter(Boolean).join(' · ');
    return (
      <RankedList
        rows={rows}
        cap={6}
        note={note}
        seeMore={{
          href: '/me/travel' as Route,
          label: (_hidden, total) => `See all ${total.toLocaleString()} →`,
        }}
      />
    );
  },
});
