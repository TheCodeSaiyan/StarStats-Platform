import React from 'react';
import type { TraceEntry } from '@/lib/api';
import { getLocationTrace } from '@/lib/api';
import { rangeToHours } from '@/lib/range';
import { logger } from '@/lib/logger';
import { toDistinctStops } from '@/components/journey/trail-utils';
import { deriveTransitionGraph } from '@/components/journey/TransitionGraph';
import { NoSignal } from '@/components/hud/NoSignal';
import { defineWidget } from './kit/defineWidget';
import { fmtNum } from './kit/format';

/**
 * `corridors` — "Top corridors": the busiest undirected A ⇄ B travel legs,
 * derived from the same me-scoped location trace the `journey` widget uses
 * (`GET /v1/me/location/trace`). No new endpoint or event type.
 *
 * The trace collapses to distinct stops, `deriveTransitionGraph` folds
 * consecutive-stop pairs into undirected edges with a trip `count`, and we
 * rank those edges. A corridor is a real observed movement between two
 * stops — never a fabricated route.
 *
 *   - compact  → just the corridor count (a bounded one-readout summary).
 *   - expanded → the top ~6 corridors, each `A ⇄ B` + count + a weight bar
 *     (share of the busiest corridor). Capped so the tile never scrolls.
 *
 * Owner-only + range-aware, mirroring `journey`: `/v1/me/location/*` has no
 * friend-scoped equivalent, so a visitor render would leak the viewer's own
 * trail. The trace window follows `ctx.range`.
 */
interface CorridorsData {
  corridors: ReadonlyArray<{ a: string; b: string; count: number }>;
  maxCount: number;
  /** Set only when the window HAS location telemetry but no travel
   *  between two distinct places — i.e. the player stayed put. Names
   *  where, so the tile reports a fact rather than an absence. Null
   *  whenever `corridors` is non-empty. */
  soleStop: string | null;
}

const TOP_N = 6;

export const corridorsWidget = defineWidget<CorridorsData>({
  id: 'corridors',
  eyebrow: 'Corridors',
  visibility: 'owner',
  rangeAware: true,
  async load(ctx) {
    // Owner-only defense: defineWidget.render() calls load() without first
    // consulting isAvailable, so guard here too — never fetch the me-scoped
    // trace for a visitor (mirrors the travel widget's load gate).
    if (!ctx.isOwner || !ctx.token) return null;
    let entries: TraceEntry[] = [];
    try {
      const trace = await getLocationTrace(ctx.token, rangeToHours(ctx.range));
      entries = trace.entries ?? [];
    } catch (err) {
      logger.warn({ err, call: 'widget.corridors' }, 'fetch failed');
      return null;
    }
    const stops = toDistinctStops(entries);
    const { nodes, edges } = deriveTransitionGraph(stops);
    if (edges.length === 0) {
      // A corridor needs TWO distinct stops with a move between them —
      // a far higher bar than "has data". Three different states used
      // to collapse to one `null` here, and the canvas then rendered
      // "No activity recorded in this window" for all of them. For a
      // player who spent the window at a single base that sentence is
      // simply untrue, and it reads as a broken widget.
      //
      // No telemetry at all IS "no activity", so keep returning null
      // and let the canvas say it. But telemetry with nowhere to go is
      // a real, different answer: name where they were.
      if (entries.length === 0) return null;
      const sole = stops[0];
      return {
        corridors: [],
        maxCount: 1,
        soleStop: sole ? (sole.resolvedLabel ?? sole.label) : null,
      };
    }
    const corridors = edges
      .map((e) => ({ a: nodes[e.a].label, b: nodes[e.b].label, count: e.count }))
      .sort((x, y) => y.count - x.count);
    const maxCount = Math.max(...corridors.map((c) => c.count), 1);
    return { corridors, maxCount, soleStop: null };
  },
  body(data, _ctx, size) {
    // Stayed in one place: report that, rather than letting the canvas
    // claim there was no activity at all.
    if (data.corridors.length === 0) {
      return (
        <NoSignal
          compact
          title="No travel this window"
          hint={
            data.soleStop
              ? `You stayed at ${data.soleStop} — a corridor needs a trip between two places.`
              : 'A corridor needs a trip between two places; none recorded in this window.'
          }
        />
      );
    }
    // Compact leads with the busiest LEG, not a bare count. A tile called
    // Corridors rendering "3 corridors" answers a question nobody asked:
    // the corridor is the datum, the count is metadata about it. The
    // total moves into the note, so nothing is lost.
    if (size === 'compact') {
      const [busiest] = data.corridors;
      const total = data.corridors.length;
      return (
        <div>
          <ul className="corr-list">
            <li className="corr">
              <span className="pair">
                {busiest.a} <span className="arr">⇄</span> {busiest.b}
              </span>
              <span className="num">{fmtNum(busiest.count)}</span>
            </li>
          </ul>
          <p className="hud-note">
            {total > 1
              ? `Busiest of ${fmtNum(total)} corridors`
              : 'Your only corridor this window'}
          </p>
        </div>
      );
    }
    const top = data.corridors.slice(0, TOP_N);
    return (
      <ul className="corr-list">
        {top.map((c, i) => (
          <li key={`${c.a}-${c.b}-${i}`} className="corr">
            <span className="pair">
              {c.a} <span className="arr">⇄</span> {c.b}
            </span>
            <span className="num">{fmtNum(c.count)}</span>
            <span className="cbar">
              <i style={{ width: `${(c.count / data.maxCount) * 100}%` }} />
            </span>
          </li>
        ))}
      </ul>
    );
  },
});
