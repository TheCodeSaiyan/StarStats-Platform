import React from 'react';
import type { Route } from 'next';
import { getFleet } from '@/lib/api';
import type { FleetResponse } from '@/lib/api';
import { loadAllReferenceBundles, type ReferenceCatalog } from '@/lib/reference';
import { EntityLink } from '@/components/kb/EntityLink';
import { logger } from '@/lib/logger';
import { rangeToWindowHours, rangeHasLifetimeBaseline } from '@/lib/range';
import { computeTrend, formatTrend, previousWindowLabel } from '@/lib/trend';
import { EmptyWindow } from './kit/EmptyWindow';
import { defineWidget } from './kit/defineWidget';
import { RankedList } from './kit/archetypes';
import { fmtNum } from './kit/format';
import { InfoTip } from '@/components/hud/InfoTip';
import { INFERENCE_EXPLANATIONS } from '@/lib/inference-explanations';

/**
 * `fleet` — "Ships you fly": top vehicle classes ranked by quantum-travel
 * trip count (`GET /v1/me/stats/fleet`). Owner-only, range-aware
 * (follows the dashboard range selector).
 *
 * Honest caveat (server holds zero RSI credentials): derived from
 * `quantum_target_selected.vehicle_class` — ships the caller has
 * quantum-travelled in, not their complete owned fleet.
 *
 * First widget migrated to the kit: `defineWidget` owns the fetch/empty/gate
 * boilerplate; `RankedList` owns the cap + "See more" (the tile never
 * scrolls — full depth lives on the entities page).
 */
interface FleetData {
  ships: ReadonlyArray<{ vehicle_class: string; trip_count: number }>;
  vehicles: ReferenceCatalog;
  /** Lifetime baseline for the windowed list's two magnitudes — how many
   *  trips, in how many ships (UX Rule 2). Null on the `all` range,
   *  which spans the whole of retention: the twin covers the same rows,
   *  so the note would only restate the list. */
  lifetime: NonNullable<FleetResponse['lifetime']> | null;
  /** Same-length window immediately before this one. Null means "no
   *  comparison to draw" -- the server omits it when the handle had no
   *  activity at all back then (not a user yet) and on `all`, which has
   *  no predecessor inside retention. NEVER coerce to 0: a real zero
   *  means "played, but none of this", which reads very differently. */
  previous: NonNullable<FleetResponse['previous']> | null;
}

/** Provenance caveat — quantum trips, not an RSI-verified owned fleet. */
const QT_CAVEAT = 'Based on quantum travel';

export const fleetWidget = defineWidget<FleetData>({
  id: 'fleet',
  eyebrow: 'Fleet',
  rangeAware: true,
  visibility: 'owner',
  async load(ctx) {
    if (!ctx.token) return null;
    let fleet = null;
    try {
      fleet = await getFleet(ctx.token, rangeToWindowHours(ctx.range));
    } catch (err) {
      logger.warn({ err, call: 'widget.fleet' }, 'fetch failed');
      return null;
    }
    const ships = fleet?.ships ?? [];
    // An empty WINDOW is not an empty account — see kit/EmptyWindow.
    const lifetimeTrips = rangeHasLifetimeBaseline(ctx.range)
      ? (fleet?.lifetime?.total_trips ?? 0)
      : 0;
    if (ships.length === 0 && lifetimeTrips === 0) return null;
    const { catalogs } = await loadAllReferenceBundles();
    return {
      ships,
      vehicles: catalogs.vehicles,
      // Lifetime is dropped on `all`: that range already spans retention,
      // so the twin covers the same rows and the note would read "N of N".
      lifetime: rangeHasLifetimeBaseline(ctx.range)
        ? (fleet?.lifetime ?? null)
        : null,
      // `previous` is NOT gated here: the server already omits it for
      // `all` and for a handle with no prior activity. Re-deciding it
      // client-side would just risk the two rules drifting apart.
      previous: fleet?.previous ?? null,
    };
  },
  body(data, ctx) {
    if (data.ships.length === 0) {
      return (
        <EmptyWindow
          rangeLabel={previousWindowLabel(ctx.range)}
          lifetimeCount={data.lifetime?.total_trips ?? 0}
          noun="quantum trips"
        />
      );
    }
    const rows = data.ships.map((s) => ({
      key: s.vehicle_class,
      label: (
        <EntityLink category="vehicle" classKey={s.vehicle_class} catalog={data.vehicles} />
      ),
      value: fmtNum(s.trip_count),
    }));
    // The window's two magnitudes against their lifetime twins. Both sides
    // come from the same top-N breakdown, so they're like-for-like. Only
    // rendered when the server sent the baseline — never substituted.
    const lifetime = data.lifetime;
    const windowTrips = data.ships.reduce((acc, s) => acc + s.trip_count, 0);
    const compare = lifetime
      ? `${fmtNum(windowTrips)} of ${fmtNum(lifetime.total_trips)} trips, ` +
        `${fmtNum(data.ships.length)} of ${fmtNum(lifetime.ships_flown)} ships all time`
      : null;
    // Trend on TRIPS only. `ships_flown` is a distinct count, so its
    // period-over-period delta is not additive and "+2 ships" would
    // read as two new ships when it may be the same pilots in a
    // different mix.
    const trend = data.previous
      ? formatTrend(
          computeTrend(windowTrips, data.previous.total_trips),
          previousWindowLabel(ctx.range),
          fmtNum,
        )
      : null;
    // The quantum-travel caveat stays: it's the honesty statement that this
    // is not the caller's owned fleet, which no comparison replaces.
    // The caveat carries an InfoTip, so the note is JSX rather than a
    // joined string — `RankedList.note` is a ReactNode.
    const note = (
      <>
        {[trend ?? compare, QT_CAVEAT].filter(Boolean).join(' · ')}
        <InfoTip label="this ranking" text={INFERENCE_EXPLANATIONS.ships_flown} />
      </>
    );
    return (
      <RankedList
        rows={rows}
        cap={6}
        note={note}
        seeMore={{
          href: `/u/${encodeURIComponent(ctx.ownerHandle)}/entities` as Route,
          label: (_hidden, total) => `See all ${total.toLocaleString()} →`,
        }}
      />
    );
  },
});
