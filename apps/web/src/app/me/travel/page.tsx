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
import { redirect } from 'next/navigation';
import { getSession } from '@/lib/session';
import {
  getMetricsEventTypes,
  getRoutes,
  getTravelStats,
  getLocationTrace,
  getLocationBreakdown,
  getCurrentLocation,
  statusOf,
  type TraceEntry,
} from '@/lib/api';
import { logger } from '@/lib/logger';
import { parseRange, rangeToHours, rangeToMetricsRange, rangeLabel } from '@/lib/range';
import { aggregateLocationBuckets } from '@/lib/class-name-parts';
import { loadAllReferenceBundles } from '@/lib/reference';
import { EntityLink } from '@/components/kb/EntityLink';
import { INFERENCE_EXPLANATIONS } from '@/lib/inference-explanations';
import { LocationChainStrip } from '@/components/journey/LocationChainStrip';
import { deriveTransitionGraph } from '@/components/journey/TransitionGraph';
import { SystemBreakdown } from '@/components/journey/SystemBreakdown';
import {
  TaxonomyStrip,
  type TaxonomyLevel,
} from './_components/TaxonomyStrip';
import Link from 'next/link';
import type { Route } from 'next';
import { PlaceDetail } from './_components/PlaceDetail';
import { buildDwellIndex } from './_components/dwell';
import { LocationHero } from '@/components/journey/LocationHero';

/**
 * Level nouns for the detail pane. Both forms are spelled out because the
 * plural is not the singular plus "s" — deriving it produced "Back to all
 * citys" on screen.
 */
const LEVEL_NOUN_FOR_CTX: Record<TaxonomyLevel, string> = {
  system: 'System',
  planet: 'Planet',
  city: 'City',
  site: 'Site',
};

const LEVEL_PLURAL: Record<TaxonomyLevel, string> = {
  system: 'systems',
  planet: 'planets',
  city: 'cities',
  site: 'sites',
};
import { TrafficMatrix } from '@/components/journey/TrafficMatrix';
import { toDistinctStops } from '@/components/journey/trail-utils';
import { countsByType, fmtNum } from '@/app/_components/widgets/kit/format';
import {
  Plane,
  MeterRow,
  SubStats,
  Flatline,
  BeamTip,
  type Calibration,
} from 'holo';
import { navSections } from '@/lib/nav';
import { getTheme } from '@/lib/theme';
import { setCalibrationAction } from '@/app/me/_projection/actions';
import {
  TravelProjection,
  type TravelSection,
} from './_projection/TravelProjection';

export const metadata = { title: 'Travel' };

interface PageProps {
  searchParams?: Promise<{ range?: string; level?: string; place?: string }>;
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
  // The taxonomy level is URL state like the range, so a chosen level is
  // shareable and survives the back button rather than being client state
  // pretending to be navigation.
  const level: TaxonomyLevel = (
    ['system', 'planet', 'city', 'site'] as const
  ).includes(sp.level as TaxonomyLevel)
    ? (sp.level as TaxonomyLevel)
    : 'system';
  const levelHref = (l: TaxonomyLevel) =>
    `/me/travel?range=${range}&level=${l}`;

  // The selected place, if any. URL state like the level and the range, so a
  // place is shareable and the back button climbs back out of it — the kit
  // holds it in `useState` because it is one mock screen.
  const place = typeof sp.place === 'string' && sp.place ? sp.place : null;
  const placeHref = (l: TaxonomyLevel, p: string) =>
    `/me/travel?range=${range}&level=${l}&place=${encodeURIComponent(p)}`;
  const hours = rangeToHours(range);

  // The beam for this render; falls back to the system default rather than
  // failing the page.
  let calibration: Calibration = 'terra';
  try {
    calibration = (await getTheme(token)) as Calibration;
  } catch {
    // Preference read failed; the default stands.
  }

  // docs/ENGINEERING.md: multi-endpoint render → Promise.allSettled. Each source
  // degrades a single section, never the whole page; each rejection logs
  // its `call=` label + status.
  const [breakdownRes, routesRes, travelRes, traceRes, dwellRes, currentRes] =
    await Promise.allSettled([
      getMetricsEventTypes(token, rangeToMetricsRange(range)),
      getRoutes(token),
      getTravelStats(token, hours),
      getLocationTrace(token, hours),
      // Real per-place dwell. The taxonomy ranked by visit count and said in a
      // comment that the product had no dwell — it does, at exactly the levels
      // the strip groups by, and this endpoint has been in `lib/api.ts` all
      // along. (`breakdownRes` above is the event-TYPE metrics call, despite
      // the name; that is what made it look already fetched.)
      getLocationBreakdown(token, hours),
      // Where the reader is NOW. A different shape from a trace entry's
      // `resolved_location` — this one carries `entered_at` / `last_seen_at`,
      // which is what the hero's live dwell ticker counts from.
      getCurrentLocation(token),
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
  if (currentRes.status === 'rejected') {
    logger.warn(
      { err: currentRes.reason, call: 'me.travel.current', status: statusOf(currentRes.reason) },
      'fetch failed',
    );
  }
  if (dwellRes.status === 'rejected') {
    logger.warn(
      { err: dwellRes.reason, call: 'me.travel.dwell', status: statusOf(dwellRes.reason) },
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
  const currentLocation =
    currentRes.status === 'fulfilled' ? currentRes.value : null;
  const dwellIndex = buildDwellIndex(
    dwellRes.status === 'fulfilled' ? dwellRes.value : null,
  );
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

  // ---------------------------------------------------------------------
  // Sections. Two groups: the ranked destinations, and the location trail.
  //
  // This page is the UNCAPPED counterpart to /me's Travel lens — every route
  // rather than the top six — which is what its "see all →" links point at.
  // ---------------------------------------------------------------------
  const sections: TravelSection[] = [
    {
      id: 'overview',
      title: 'Quantum travel',
      ctx: rangeLabel(range),
      group: 'routes',
      node: (
        <>
          <SubStats
            items={[
              { k: 'Quantum', v: fmtNum(quantums) },
              { k: 'Server hops', v: fmtNum(serverHops) },
              ...(planets > 0
                ? [{ k: 'Planets', v: fmtNum(planets) }]
                : []),
            ]}
          />
          {/* Both headline figures are INFERRED from log lines rather than
              logged as such, and the system's rule is that an inferred metric
              carries its derivation rather than rounding the caveat away. */}
          <p className="hp-prose">
            <BeamTip
              note={INFERENCE_EXPLANATIONS.quantum_jumps}
              label="How quantum jumps are derived"
            >
              Quantum jumps
            </BeamTip>{' '}
            and{' '}
            <BeamTip
              note={INFERENCE_EXPLANATIONS.server_hops}
              label="How server hops are derived"
            >
              server hops
            </BeamTip>{' '}
            are both inferred from the log.
          </p>
        </>
      ),
    },

    ...(!hasAnyTravel
      ? [
          {
            id: 'empty',
            title: 'No travel in this window',
            group: 'routes',
            node: <Flatline reason="no-telemetry" />,
          } satisfies TravelSection,
        ]
      : []),

    ...(hasAnyTravel && routeRows.length > 0
      ? [
          {
            id: 'routes',
            title: 'All routes',
            ctx: `${routeRows.length.toLocaleString()} · by trips`,
            group: 'routes',
            node: (
              <Plane
                tilt="flat"
                cap="Quantum destinations"
                hint="uncapped"
                style={{ marginTop: 18 }}
              >
                {/* UNCAPPED on purpose — the lens shows six and links here for
                    the rest. A cap on the detail page would leave the reader
                    with nowhere further to go. */}
                {routeRows.map((r, i) => (
                  <MeterRow
                    key={r.key}
                    rank={i + 1}
                    name={r.label}
                    value={fmtNum(r.count)}
                    pct={(r.count / maxRouteCount) * 100}
                  />
                ))}
              </Plane>
            ),
          } satisfies TravelSection,
        ]
      : []),

    ...(hasAnyTravel && entries.length > 0
      ? [
          {
            id: 'trail',
            title: 'Location trail',
            ctx: rangeLabel(range),
            group: 'trail',
            node: (
              <>
                {/* WHERE YOU ARE NOW, restored.
                    `LocationHero` — the System > Planet > City breadcrumb, the
                    live dwell ticker counting up from the last stop, and the
                    "came from" trail — was built for `/journey?view=location`
                    and has rendered NOWHERE since that route became a redirect
                    to `/me`. It was dead code on `next` before this port, not
                    something the port dropped, and it is the most specific
                    answer this page can give to "where am I": everything else
                    here is an aggregate over a window. */}
                <LocationHero
                  location={currentLocation}
                  stops={stops}
                  catalog={locations}
                />
                <LocationChainStrip
                  entries={entries}
                  eyebrow="Recent stops"
                  catalog={locations}
                />
                {series.length >= 2 ? (
                  <Plane
                    tilt="flat"
                    cap="Daily activity"
                    hint={rangeLabel(range)}
                    style={{ marginTop: 20 }}
                  >
                    {/* Real summed event counts per calendar day, not a
                        fabricated trend — height and brightness, never hue. */}
                    <div
                      className="hp-spark"
                      aria-label={`Location activity by day, ${rangeLabel(range)}`}
                    >
                      {series.map((n, i) => (
                        <i
                          key={i}
                          title={`${n} event${n === 1 ? '' : 's'}`}
                          style={{
                            height: `${Math.max(2, (n / Math.max(...series, 1)) * 100)}%`,
                          }}
                        />
                      ))}
                    </div>
                  </Plane>
                ) : null}
              </>
            ),
          } satisfies TravelSection,
        ]
      : []),

    ...(hasAnyTravel && entries.length > 0
      ? [
          {
            id: 'systems',
            title: place ?? 'Where you have been',
            ctx: place
              ? `${LEVEL_NOUN_FOR_CTX[level]} · charted`
              : 'By taxonomy level',
            group: 'trail',
            // `Journey.jsx` browses locations through a category strip —
            // Systems / Planets / Cities / Sites — where this page had one
            // fixed level. The system breakdown stays below it: it answers a
            // different question (how the window splits across systems) and is
            // the one level with a share worth drawing.
            node: (
              <>
                <TaxonomyStrip
                  stops={stops}
                  level={level}
                  buildHref={levelHref}
                  buildPlaceHref={(p) => placeHref(level, p)}
                  dwell={dwellIndex}
                />
                {place ? (
                  <>
                    <p className="hp-prose" style={{ marginTop: 18 }}>
                      <Link href={levelHref(level) as Route}>
                        &larr; Back to all {LEVEL_PLURAL[level]}
                      </Link>
                    </p>
                    <PlaceDetail
                      stops={stops}
                      level={level}
                      place={place}
                      buildChildHref={placeHref}
                      dwell={dwellIndex}
                    />
                  </>
                ) : (
                  <SystemBreakdown entries={entries} catalog={locations} />
                )}
              </>
            ),
          } satisfies TravelSection,
          {
            id: 'matrix',
            title: 'Traffic matrix',
            ctx: 'Between stops',
            group: 'trail',
            node: <TrafficMatrix graph={deriveTransitionGraph(stops)} />,
          } satisfies TravelSection,
        ]
      : []),
  ];

  return (
    <TravelProjection
      handle={session.claimedHandle}
      calibration={calibration}
      range={range}
      nav={navSections(
        { signedIn: true, staffRoles: session.staffRoles },
        'travel',
      )}
      sections={sections}
      notice={null}
      onCalibrate={async (id: string) => {
        'use server';
        await setCalibrationAction(id);
      }}
    />
  );
}
