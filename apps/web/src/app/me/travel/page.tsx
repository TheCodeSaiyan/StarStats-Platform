/**
 * `/me/travel` — the owner-only travel detail page.
 *
 * The real destination behind every "See more" / "See travel map" link on
 * the travel / routes / locations widgets (which used to point at the now-
 * deprecated `/journey` redirect stub). Composes, on ONE page:
 *
 *   1. Travel stats header — quantum jumps, server hops, planets visited
 *      (mirrors the `travel` widget's derivation: `getTravelStats` for the
 *      server-computed aggregates, `getMetricsEventTypes` for the server-hop
 *      event counts).
 *   2. All routes — the FULL ranked list of quantum destinations
 *      (`getRoutes` → `aggregateLocationBuckets`), NOT capped, each deep-
 *      linked to the KB via `<EntityLink>`.
 *   3. Travel map / trail — the journey widget's expanded composition
 *      (`getLocationTrace` → recent-stops strip + transition graph +
 *      activity heatmap + full timeline).
 *
 * Owner-only: every source is me-scoped (`/v1/me/*`) with no friend
 * equivalent — same gate as the `travel` / `journey` widgets. A visitor
 * render would surface the VIEWER's own data. Signed-out → login redirect.
 *
 * Range-aware: reads `?range=` (defaults to {@link DEFAULT_RANGE} via
 * `parseRange`) and drives the trace / travel-stats windows off it, exactly
 * like the widgets. A `<RangeBar>` re-navigates the page per range.
 *
 * Fetching follows the docs/ENGINEERING.md invariant: every multi-endpoint render uses
 * `Promise.allSettled`, never `Promise.all`, so one endpoint hiccup degrades
 * a single section rather than blanking the page. Each rejection is logged
 * individually with its `call=` label.
 */

import 'server-only';
import React from 'react';
import Link from 'next/link';
import type { Route } from 'next';
import { redirect } from 'next/navigation';
import { getSession } from '@/lib/session';
import {
  getMetricsEventTypes,
  getRoutes,
  getTravelStats,
  getLocationTrace,
  statusOf,
  type TraceEntry,
} from '@/lib/api';
import { logger } from '@/lib/logger';
import { parseRange, rangeToHours, rangeToMetricsRange, rangeLabel } from '@/lib/range';
import { RangeBar } from '@/components/journey/RangeBar';
import { aggregateLocationBuckets } from '@/lib/class-name-parts';
import { loadAllReferenceBundles } from '@/lib/reference';
import { EntityLink } from '@/components/kb/EntityLink';
import { InfoTip } from '@/components/hud/InfoTip';
import { INFERENCE_EXPLANATIONS } from '@/lib/inference-explanations';
import { NoSignal } from '@/components/hud/NoSignal';
import { Sparkline } from '@/components/metrics/Sparkline';
import { LocationChainStrip } from '@/components/journey/LocationChainStrip';
import { LocationTimeline } from '@/components/journey/LocationTimeline';
import { LocationActivityHeatmap } from '@/components/journey/LocationActivityHeatmap';
import { deriveTransitionGraph } from '@/components/journey/TransitionGraph';
import { SystemBreakdown } from '@/components/journey/SystemBreakdown';
import { TrafficMatrix } from '@/components/journey/TrafficMatrix';
import { toDistinctStops } from '@/components/journey/trail-utils';
import { ReadoutGroup, type Readout } from '@/app/_components/widgets/kit/archetypes';
import { countsByType, fmtNum } from '@/app/_components/widgets/kit/format';

export const metadata = { title: 'Travel' };

interface PageProps {
  searchParams?: Promise<{ range?: string }>;
}

/** Honest daily-activity series from the trace: total event_count per
 *  calendar day (browser wall-clock, matching the heatmap), chronological
 *  oldest → newest. Lifted verbatim from the `journey` widget — each point
 *  is a real summed event count, not a fabricated trend. */
function dailyActivitySeries(entries: TraceEntry[]): number[] {
  const byDay = new Map<number, number>();
  for (const e of entries) {
    const d = new Date(e.started_at);
    if (Number.isNaN(d.getTime())) continue;
    const dayStart = new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
    byDay.set(dayStart, (byDay.get(dayStart) ?? 0) + e.event_count);
  }
  return [...byDay.entries()]
    .sort((a, b) => a[0] - b[0])
    .map(([, count]) => count);
}

export default async function TravelPage(props: PageProps) {
  const session = await getSession();
  if (!session) redirect('/auth/login?next=/me/travel');

  const token = session.token;
  const sp = props.searchParams ? await props.searchParams : {};
  const range = parseRange(sp.range);
  const hours = rangeToHours(range);

  // docs/ENGINEERING.md: multi-endpoint render → Promise.allSettled. Each source
  // degrades a single section, never the whole page; each rejection logs
  // its `call=` label + status.
  const [breakdownRes, routesRes, travelRes, traceRes] = await Promise.allSettled([
    getMetricsEventTypes(token, rangeToMetricsRange(range)),
    getRoutes(token),
    getTravelStats(token, hours),
    getLocationTrace(token, hours),
  ]);

  if (breakdownRes.status === 'rejected') {
    logger.warn(
      { err: breakdownRes.reason, call: 'me.travel.breakdown', status: statusOf(breakdownRes.reason) },
      'fetch failed',
    );
  }
  if (routesRes.status === 'rejected') {
    logger.warn(
      { err: routesRes.reason, call: 'me.travel.routes', status: statusOf(routesRes.reason) },
      'fetch failed',
    );
  }
  if (travelRes.status === 'rejected') {
    logger.warn(
      { err: travelRes.reason, call: 'me.travel.stats', status: statusOf(travelRes.reason) },
      'fetch failed',
    );
  }
  if (traceRes.status === 'rejected') {
    logger.warn(
      { err: traceRes.reason, call: 'me.travel.trace', status: statusOf(traceRes.reason) },
      'fetch failed',
    );
  }

  const breakdown = breakdownRes.status === 'fulfilled' ? breakdownRes.value : null;
  const travelStats = travelRes.status === 'fulfilled' ? travelRes.value : null;
  const routes = routesRes.status === 'fulfilled' ? routesRes.value?.routes ?? [] : [];
  const entries: TraceEntry[] = traceRes.status === 'fulfilled' ? traceRes.value.entries ?? [] : [];

  // Mirror the `travel` widget's stat derivation: prefer the server-computed
  // quantum aggregate, fall back to the raw target-selection count; server
  // hops = join_pu + change_server; planets = distinct planets visited.
  const counts = countsByType(breakdown?.types);
  const quantums = travelStats?.quantum_jumps ?? counts['quantum_target_selected'] ?? 0;
  const serverHops = (counts['join_pu'] ?? 0) + (counts['change_server'] ?? 0);
  const planets = travelStats?.planets_visited?.length ?? 0;

  // Locations catalog for KB deep-links (dual-keyed by display_name).
  const { catalogs } = await loadAllReferenceBundles();
  const locations = catalogs.locations;

  const readouts: Readout[] = [
    {
      label: 'quantum',
      info: <InfoTip label="quantum jumps" text={INFERENCE_EXPLANATIONS.quantum_jumps} />,
      value: fmtNum(quantums),
    },
    {
      label: 'server hops',
      info: <InfoTip label="server hops" text={INFERENCE_EXPLANATIONS.server_hops} />,
      value: fmtNum(serverHops),
    },
    ...(planets > 0 ? [{ label: 'planets', value: fmtNum(planets) } as Readout] : []),
  ];

  // The FULL routes list — resolve raw engine ids / pipe hierarchies to
  // friendly labels and merge duplicates, then deep-link each. `label` is
  // pinned so the class-id prettifier never rewrites a free-text
  // destination (docs/ENGINEERING.md free-text rule). NOT capped: this is the
  // detail page the widget's "See all →" points at.
  const routeRows = aggregateLocationBuckets(
    routes.map((r) => ({ value: r.destination, count: r.count })),
  ).map((a) => ({
    key: a.label,
    label: (
      <EntityLink category="location" classKey={a.label} catalog={locations} label={a.label} />
    ),
    // Carry the raw count too — the ranked leaderboard draws a share-of-
    // busiest meter from it, not just the formatted value.
    count: a.count,
  }));
  // Busiest destination drives the underline meter widths.
  const maxRouteCount = Math.max(...routeRows.map((r) => r.count), 1);

  const stops = toDistinctStops(entries);
  const series = dailyActivitySeries(entries);
  const hasAnyTravel = routeRows.length > 0 || entries.length > 0;

  return (
    // `role="main"` on a DIV (not a <main> element) so the global
    // `main {}` 720px legacy column in globals.css doesn't clamp this
    // full-width detail page — same landmark for AT, zero CSS collision.
    <div
      role="main"
      className="ss-screen-enter"
      style={{ display: 'flex', flexDirection: 'column', gap: 12 }}
    >
      {/* Command-bar header: page identity on the left, controls right —
          mirrors the /me focus strip (ss-eyebrow + .hud-controls chips). */}
      <header
        style={{ display: 'flex', alignItems: 'flex-end', gap: 12, flexWrap: 'wrap' }}
      >
        <div>
          <div className="ss-eyebrow">Travel</div>
          <h1
            style={{ margin: '3px 0 0', fontSize: 24, fontWeight: 600, letterSpacing: '-0.02em' }}
          >
            Travel map
          </h1>
        </div>
        <div className="hud-controls" style={{ marginLeft: 'auto', marginBottom: 0 }}>
          <Link href={'/me' as Route} className="hud-chip">
            ← Dashboard
          </Link>
          <RangeBar active={range} buildHref={(id) => `/me/travel?range=${id}` as Route} />
        </div>
      </header>

      {/* Overview — quantum / hops / planets as HUD readouts */}
      <section className="hud-tile">
        <div className="hud-tile__hd">
          <span className="hud-tile__eyebrow">Quantum travel</span>
          <span className="hud-tile__title">Overview</span>
          <span className="hud-tile__sub">{rangeLabel(range)}</span>
        </div>
        <div className="hud-tile__body" style={{ marginTop: 4 }}>
          <ReadoutGroup readouts={readouts} />
        </div>
      </section>

      {!hasAnyTravel ? (
        <section className="hud-tile">
          <div className="hud-tile__body">
            <NoSignal reason="no-telemetry" />
          </div>
        </section>
      ) : (
        <>
          {/* All routes — full, uncapped ranked list */}
          {routeRows.length > 0 && (
            <section className="hud-tile">
              <div className="hud-tile__hd">
                <span className="hud-tile__eyebrow">Quantum destinations</span>
                <span className="hud-tile__title">All routes</span>
                <span className="hud-tile__sub">
                  {routeRows.length.toLocaleString()} · by trips
                </span>
              </div>
              <div className="hud-tile__body" style={{ marginTop: 4 }}>
                {/* Ranked leaderboard — 2-digit mono rank, destination
                    (EntityLink) with an underline meter beneath, trip count
                    right. Uncapped: this is the detail page. */}
                <ul className="tl-lead-list">
                  {routeRows.map((r, i) => (
                    <li key={r.key} className="tl-lead">
                      <span
                        className={
                          i === 0 ? 'tl-lead__rank tl-lead__rank--top' : 'tl-lead__rank'
                        }
                      >
                        {String(i + 1).padStart(2, '0')}
                      </span>
                      <span className="tl-lead__lab">
                        {r.label}
                        <small
                          className="tl-lead__meter"
                          style={{ width: `${(r.count / maxRouteCount) * 100}%` }}
                        />
                      </span>
                      <span className="tl-lead__num">{fmtNum(r.count)}</span>
                    </li>
                  ))}
                </ul>
              </div>
            </section>
          )}

          {/* Journey — the location trail: recent stops, map, activity, timeline */}
          {entries.length > 0 && (
            <section className="hud-tile">
              <div className="hud-tile__hd">
                <span className="hud-tile__eyebrow">Location trail</span>
                <span className="hud-tile__title">Journey</span>
                <span className="hud-tile__sub">{rangeLabel(range)}</span>
              </div>
              <div
                className="hud-tile__body"
                style={{ display: 'flex', flexDirection: 'column', gap: 16, marginTop: 8 }}
              >
                <LocationChainStrip
                  entries={entries}
                  maxStops={6}
                  eyebrow="Recent stops"
                  catalog={locations}
                />

                {series.length >= 2 && (
                  <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
                    <Sparkline
                      series={series}
                      label={`Location activity by day, ${rangeLabel(range)}`}
                      width={160}
                      height={28}
                    />
                    <span className="hud-note" style={{ margin: 0 }}>
                      Daily activity · {rangeLabel(range)}
                    </span>
                  </div>
                )}

                <div>
                  <div className="hud-tile__eyebrow" style={{ marginBottom: 6 }}>
                    System breakdown
                  </div>
                  <SystemBreakdown entries={entries} catalog={locations} />
                </div>

                <div>
                  <div className="hud-tile__eyebrow" style={{ marginBottom: 6 }}>
                    Traffic matrix
                  </div>
                  <TrafficMatrix graph={deriveTransitionGraph(stops)} />
                </div>

                <div>
                  <div className="hud-tile__eyebrow" style={{ marginBottom: 6 }}>
                    When you played
                  </div>
                  <LocationActivityHeatmap entries={entries} windowHours={hours} />
                </div>

                <div>
                  <div className="hud-tile__eyebrow" style={{ marginBottom: 6 }}>
                    Stops
                  </div>
                  <LocationTimeline entries={entries} catalog={locations} />
                </div>
              </div>
            </section>
          )}
        </>
      )}
    </div>
  );
}
