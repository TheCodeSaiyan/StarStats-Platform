import React from 'react';
import Link from 'next/link';
import type { Route } from 'next';
import { getMetricsEventTypes, getRoutes, getTravelStats } from '@/lib/api';
import { InfoTip } from '@/components/hud/InfoTip';
import { INFERENCE_EXPLANATIONS } from '@/lib/inference-explanations';
import { rangeToMetricsRange, rangeToHours } from '@/lib/range';
import { prettyLocationLabel } from '@/lib/class-name-parts';
import { loadAllReferenceBundles, type ReferenceCatalog } from '@/lib/reference';
import { logger } from '@/lib/logger';
import { defineWidget } from './kit/defineWidget';
import { ReadoutGroup, type Readout } from './kit/archetypes';
import { fmtNum, countsByType, sumCounts } from './kit/format';

/**
 * `travel` — quantum jumps, server hops, planets, and top routes.
 *
 * REBUILT (widgets v2): the expanded view used to dump a raw event-type
 * breakdown ("Joined PU / Changed server / Spawned…") that read like a raw
 * log, plus an inline taxonomy map that overflowed the tile. Per the owner's
 * rule — never scroll, "See more" for depth — expanded now shows the same
 * meaningful travel metrics as compact (fuller) + the top routes, and links
 * the full travel map to /journey. No raw event dump, no clipped map.
 *
 * Owner-only: the only source is me-scoped `/v1/me/metrics/event-types`
 * (no friend endpoint) — a visitor path would surface the VIEWER's data.
 */
const TRAVEL_TYPES = [
  'join_pu',
  'change_server',
  'seed_solar_system',
  'resolve_spawn',
  'quantum_target_selected',
  'planet_terrain_load',
  'vehicle_stowed',
];

interface TravelData {
  quantums: number;
  serverHops: number;
  planets: number;
  routes: ReadonlyArray<{ destination: string; count: number }>;
  locations: ReferenceCatalog;
}

export const travelWidget = defineWidget<TravelData>({
  id: 'travel',
  eyebrow: 'Travel',
  rangeAware: true,
  isAvailable: (ctx) => ctx.isOwner,
  async load(ctx) {
    if (!ctx.isOwner || !ctx.token) return null;
    const token = ctx.token;
    const hours = rangeToHours(ctx.range);
    // Routes take the SAME `hours` window as travel stats: fetching them
    // unscoped listed lifetime top routes beside range-scoped quantum-jump
    // and server-hop counts under a single range label.
    const [breakdownRes, routesRes, travelRes] = await Promise.allSettled([
      getMetricsEventTypes(token, rangeToMetricsRange(ctx.range)),
      getRoutes(token, hours),
      getTravelStats(token, hours),
    ]);
    if (breakdownRes.status === 'rejected') {
      logger.warn({ err: breakdownRes.reason, call: 'widget.travel' }, 'fetch failed');
    }
    if (routesRes.status === 'rejected') {
      logger.warn({ err: routesRes.reason, call: 'widget.travel.routes' }, 'fetch failed');
    }
    if (travelRes.status === 'rejected') {
      logger.warn({ err: travelRes.reason, call: 'widget.travel.stats' }, 'fetch failed');
    }
    const breakdown = breakdownRes.status === 'fulfilled' ? breakdownRes.value : null;
    if (!breakdown) return null;
    const counts = countsByType(breakdown.types);
    if (sumCounts(breakdown.types, TRAVEL_TYPES) === 0) return null;
    const travelStats = travelRes.status === 'fulfilled' ? travelRes.value : null;
    const routes = routesRes.status === 'fulfilled' ? routesRes.value?.routes ?? [] : [];
    const { catalogs } = await loadAllReferenceBundles();
    return {
      // Prefer the server-computed quantum-jump aggregate; fall back to the
      // raw target-selection count when travel-stats degraded.
      quantums: travelStats?.quantum_jumps ?? counts['quantum_target_selected'] ?? 0,
      serverHops: (counts['join_pu'] ?? 0) + (counts['change_server'] ?? 0),
      // A REAL COUNT now. This was `planets_visited?.length` — the number of
      // BUCKETS the server returned, capped at 100, so the figure pinned there
      // and stopped reporting. The server returns the distinct count itself.
      planets: travelStats?.distinct_planets ?? 0,
      routes,
      locations: catalogs.locations,
    };
  },
  body(data, _ctx, size) {
    const readouts: Readout[] = [
      {
        label: 'quantum',
        info: <InfoTip label="quantum jumps" text={INFERENCE_EXPLANATIONS.quantum_jumps} />,
        value: fmtNum(data.quantums),
      },
      {
        label: 'server hops',
        info: <InfoTip label="server hops" text={INFERENCE_EXPLANATIONS.server_hops} />,
        value: fmtNum(data.serverHops),
      },
      ...(data.planets > 0
        ? [{ label: 'planets', value: fmtNum(data.planets) } as Readout]
        : []),
    ];

    if (size === 'compact') {
      const top = data.routes[0];
      return (
        <ReadoutGroup
          readouts={readouts}
          note={
            top
              ? `Top route: ${prettyLocationLabel(top.destination)} (${fmtNum(top.count)})`
              : undefined
          }
        />
      );
    }

    // NO ROUTE LIST HERE, at either size. Expanded used to render
    // `aggregateLocationBuckets(data.routes)` as an EntityLink'd RankedList —
    // the same data, the same transform and the same rendering as the `routes`
    // widget, off the same getRoutes call, in the same lens. Two tiles showing
    // one list.
    //
    // `routes` is the better of the two: it carries the window's trips against
    // their lifetime total and a period trend, and deliberately withholds
    // `lifetime.destinations` because that counts RAW destinations while its
    // rows are MERGED buckets. This tile keeps what only it reports — quantum,
    // server hops, planets — names the top route in its note, and sends depth
    // to the map rather than restating a neighbour.
    const top = data.routes[0];
    return (
      <div>
        <ReadoutGroup
          readouts={readouts}
          note={
            top
              ? `Top route: ${prettyLocationLabel(top.destination)} (${fmtNum(top.count)})`
              : undefined
          }
        />
        <p className="hud-note">
          <Link href={'/me/travel' as Route}>See travel map →</Link>
        </p>
      </div>
    );
  },
});
