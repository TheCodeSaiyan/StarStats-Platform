import React from 'react';
import type { TraceEntry } from '@/lib/api';
import { getLocationTrace, getLocationBreakdown } from '@/lib/api';
import { rangeToHours, rangeLabel } from '@/lib/range';
import { loadAllReferenceBundles } from '@/lib/reference';
import { logger } from '@/lib/logger';
import { NoSignal } from '@/components/hud/NoSignal';
import { Sparkline } from '@/components/metrics/Sparkline';
import { LocationChainStrip } from '@/components/journey/LocationChainStrip';
import { LocationTimeline } from '@/components/journey/LocationTimeline';
import { LocationActivityHeatmap } from '@/components/journey/LocationActivityHeatmap';
import { LocationTypicalInsight } from '@/components/journey/LocationTypicalInsight';
import { TransitionGraph } from '@/components/journey/TransitionGraph';
import { toDistinctStops } from '@/components/journey/trail-utils';
import type { WidgetDef } from './types';

/**
 * `journey` widget — the owner's recent location trail rendered as a
 * route map. Surfaces EXISTING me-scoped location telemetry only; no
 * new event types or endpoints.
 *
 *   - compact  -> `LocationChainStrip` (last few distinct stops)
 *   - expanded -> a schematic transition graph, an honest daily-activity
 *     sparkline, the full timeline, an hour-of-day heatmap and a
 *     "recent vs typical" one-liner.
 *
 * Owner-only: every source (`/v1/me/location/*`) is me-scoped with no
 * friend equivalent, so rendering for a visitor would leak the VIEWER's
 * own trail onto the owner's profile — same gate as `travel`. Range-
 * aware: the trace/breakdown windows follow `ctx.range`.
 *
 * Returns body-only content; the tile shell + eyebrow + title come from
 * the canvas. Never returns a blank box — an empty window degrades to
 * `<NoSignal>` with the honest "no location telemetry" reason.
 */

/** Honest daily-activity series from the trace: total event_count per
 *  calendar day (browser wall-clock, matching the heatmap), chronological
 *  oldest -> newest. This IS a real series (not a fabricated trend) —
 *  each point is the summed event count for that day. */
function dailyActivitySeries(entries: TraceEntry[]): number[] {
  const byDay = new Map<number, number>();
  for (const e of entries) {
    const d = new Date(e.started_at);
    if (Number.isNaN(d.getTime())) continue;
    const dayStart = new Date(
      d.getFullYear(),
      d.getMonth(),
      d.getDate(),
    ).getTime();
    byDay.set(dayStart, (byDay.get(dayStart) ?? 0) + e.event_count);
  }
  return [...byDay.entries()]
    .sort((a, b) => a[0] - b[0])
    .map(([, count]) => count);
}

export const journeyWidget: WidgetDef = {
  id: 'journey',
  defaultSize: 'compact',
  eyebrow: 'Journey',
  rangeAware: true,
  isAvailable(ctx) {
    // Owner-only: /v1/me/location/* has no friend-scoped equivalent, so
    // a visitor render would surface the viewer's own trail. Mirrors the
    // travel widget gate. Do NOT add a visitor path without a real
    // /v1/u/{handle}/location endpoint.
    return ctx.isOwner;
  },
  async render(ctx, size) {
    if (!ctx.isOwner || !ctx.token) return null;
    const token = ctx.token;
    const hours = rangeToHours(ctx.range);

    if (size === 'compact') {
      let entries: TraceEntry[] = [];
      try {
        const trace = await getLocationTrace(token, hours);
        entries = trace.entries ?? [];
      } catch (err) {
        logger.warn({ err, call: 'widget.journey.trace' }, 'fetch failed');
      }
      if (entries.length === 0) {
        return <NoSignal compact reason="no-telemetry" />;
      }
      // Thread the locations catalog so each stop deep-links to the KB
      // (the chain-strip already wraps labels in <EntityLink>; without a
      // catalog they degrade to plain text). Memoised in-process.
      const { catalogs } = await loadAllReferenceBundles();
      return (
        <LocationChainStrip
          entries={entries}
          maxStops={4}
          eyebrow="Recent stops"
          catalog={catalogs.locations}
        />
      );
    }

    // Expanded: the range-scoped trace drives the timeline / heatmap /
    // transition graph / sparkline. The "recent vs typical" insight is NOT
    // range-aware — its wording ("today" vs "typical week") demands its own
    // FIXED windows (24h trace + 7d breakdown), otherwise a wide range makes
    // both sides the same period and the "today" claim is false. Each fetch
    // is independent; one failure degrades a section, not the whole widget.
    const TODAY_HOURS = 24;
    const TYPICAL_HOURS = 24 * 7;
    const [traceRes, todayTraceRes, typicalBreakdownRes] = await Promise.allSettled([
      getLocationTrace(token, hours),
      getLocationTrace(token, TODAY_HOURS),
      getLocationBreakdown(token, TYPICAL_HOURS),
    ]);
    if (traceRes.status === 'rejected') {
      logger.warn({ err: traceRes.reason, call: 'widget.journey.trace' }, 'fetch failed');
    }
    if (todayTraceRes.status === 'rejected') {
      logger.warn({ err: todayTraceRes.reason, call: 'widget.journey.today' }, 'fetch failed');
    }
    if (typicalBreakdownRes.status === 'rejected') {
      logger.warn(
        { err: typicalBreakdownRes.reason, call: 'widget.journey.typical' },
        'fetch failed',
      );
    }
    const entries: TraceEntry[] =
      traceRes.status === 'fulfilled' ? traceRes.value.entries ?? [] : [];
    const todayStops = toDistinctStops(
      todayTraceRes.status === 'fulfilled' ? todayTraceRes.value.entries ?? [] : [],
    );
    const typicalBreakdown =
      typicalBreakdownRes.status === 'fulfilled' ? typicalBreakdownRes.value : null;

    if (entries.length === 0) {
      // Compact even in the expanded size: an empty widget should never be a
      // full-height box (content auto-fit collapses this to a short tile).
      return <NoSignal compact reason="no-telemetry" />;
    }

    const stops = toDistinctStops(entries);
    const series = dailyActivitySeries(entries);
    // Locations catalog for KB deep-links in the timeline rows (the
    // TransitionGraph is an SVG schematic and stays text-only).
    const { catalogs } = await loadAllReferenceBundles();

    return (
      <div style={{ display: 'flex', flexDirection: 'column', gap: 16 }}>
        {series.length >= 2 && (
          <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
            <Sparkline
              series={series}
              label={`Location activity by day, ${rangeLabel(ctx.range)}`}
              width={140}
              height={28}
            />
            <span className="hud-note" style={{ margin: 0 }}>
              Daily activity · {rangeLabel(ctx.range)}
            </span>
          </div>
        )}

        <div>
          <div className="hud-tile__eyebrow" style={{ marginBottom: 6 }}>
            Route map
          </div>
          <TransitionGraph stops={stops} />
        </div>

        <LocationTypicalInsight todayStops={todayStops} typicalBreakdown={typicalBreakdown} />

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
          <LocationTimeline entries={entries} catalog={catalogs.locations} />
        </div>
      </div>
    );
  },
};
