/**
 * Schematic origin -> destination transition graph.
 *
 * The server carries NO spatial coordinates for in-game locations
 * (confirmed — nothing in the trace/breakdown/stats responses exposes
 * x/y/z or a map projection). So this is deliberately a SCHEMATIC, not a
 * star map: nodes are laid out in a single vertical column ordered by
 * FIRST-VISIT time (from `started_at`), and the arcs between them encode
 * how often the player moved between two stops. The y-position means
 * "order of appearance", never distance — we don't fake geometry we
 * don't have.
 *
 * Data derivation (pure, unit-tested via `deriveTransitionGraph`):
 *   - nodes  = distinct stops, `visits` = number of arrivals at that stop
 *   - edges  = consecutive stop pairs collapsed to an UNDIRECTED pair,
 *              `count` = number of trips between the two stops (either way)
 *
 * Server component — hook-free, no `'use client'`, so it renders inside
 * an async widget body. Styled with design-token CSS variables inline
 * (matching the sibling journey components), so no new stylesheet class
 * is required.
 */

import React from 'react';
import type { DistinctStop } from './trail-utils';

export interface GraphNode {
  key: string;
  label: string;
  /** Number of times the player arrived at this stop in the window. */
  visits: number;
  /** Order index (0 = earliest first-visit). Drives vertical position. */
  order: number;
}

export interface GraphEdge {
  /** Index into the ordered `nodes` array. `a < b` always. */
  a: number;
  b: number;
  /** Trips between the two stops, summed across both directions. */
  count: number;
}

export interface TransitionGraph {
  nodes: GraphNode[];
  edges: GraphEdge[];
}

/**
 * Fold a chronological (oldest -> newest) distinct-stop list into a
 * schematic transition graph. `maxNodes` caps the node set to the most-
 * visited stops so the diagram stays legible; edges touching a dropped
 * node are discarded.
 */
export function deriveTransitionGraph(
  rawStops: DistinctStop[],
  maxNodes = 9,
): TransitionGraph {
  // Drop stops that name no place, then re-collapse.
  //
  // `join_pu` is in LOCATION_EVENT_TYPES so it reaches the trace, but
  // `resolve_join_pu` sets planet/city/system ALL null on purpose — it
  // carries shard identity, not a location. `toDistinctStops` does not
  // filter those, so every server hop became a stop labelled "In
  // transit" (primaryLabel's last fallback) under the key `'||'`.
  //
  // Measured on a real 24h window: 11 join_pu among 19 collapsed stops.
  // Half the graph was phantom and the busiest "corridor" came out as
  // `In transit ⇄ microTech`. A 7d window holds proportionally far fewer
  // hops, which is why SHORT windows looked worst.
  //
  // Nobody visits a place called In transit. A hop between two real
  // stops must not break the corridor between them — so the pair either
  // side of it becomes adjacent, and identical neighbours must
  // re-collapse or the graph gains a self-edge and a phantom visit.
  const stops: DistinctStop[] = [];
  for (const s of rawStops) {
    if (s.system === null && s.planet === null && s.city === null) continue;
    if (stops.length > 0 && stops[stops.length - 1].key === s.key) continue;
    stops.push(s);
  }
  if (stops.length === 0) return { nodes: [], edges: [] };

  // Distinct nodes, keyed by location identity. `visits` counts arrivals
  // (one per run in the collapsed stop list); `firstOrder` preserves the
  // chronological first-appearance so layout ordering comes from time.
  const byKey = new Map<
    string,
    { label: string; visits: number; firstOrder: number }
  >();
  stops.forEach((s, i) => {
    const cur = byKey.get(s.key);
    if (cur) {
      cur.visits += 1;
    } else {
      byKey.set(s.key, {
        label: s.resolvedLabel ?? s.label,
        visits: 1,
        firstOrder: i,
      });
    }
  });

  // Keep the most-visited stops (ties broken by earlier first-visit), then
  // re-sort the survivors back into chronological order for the layout.
  const kept = [...byKey.entries()]
    .sort((a, b) => b[1].visits - a[1].visits || a[1].firstOrder - b[1].firstOrder)
    .slice(0, maxNodes)
    .sort((a, b) => a[1].firstOrder - b[1].firstOrder);

  const indexByKey = new Map<string, number>();
  const nodes: GraphNode[] = kept.map(([key, v], order) => {
    indexByKey.set(key, order);
    return { key, label: v.label, visits: v.visits, order };
  });

  // Consecutive pairs -> undirected edges. Self-pairs can't occur (the
  // stop collapser already merged repeats), but guard anyway.
  const edgeMap = new Map<string, number>();
  for (let i = 0; i < stops.length - 1; i++) {
    const from = indexByKey.get(stops[i].key);
    const to = indexByKey.get(stops[i + 1].key);
    if (from === undefined || to === undefined || from === to) continue;
    const a = Math.min(from, to);
    const b = Math.max(from, to);
    const k = `${a}-${b}`;
    edgeMap.set(k, (edgeMap.get(k) ?? 0) + 1);
  }
  const edges: GraphEdge[] = [...edgeMap.entries()].map(([k, count]) => {
    const [a, b] = k.split('-').map(Number);
    return { a, b, count };
  });

  return { nodes, edges };
}

interface Props {
  /** Chronological distinct stops (oldest -> newest). */
  stops: DistinctStop[];
  /** Cap on rendered nodes. Default 9. */
  maxNodes?: number;
}

const LABEL_W = 150;
const NODE_X = LABEL_W + 8;
const ROW_H = 34;
const PAD_Y = 16;
const VIEW_W = 340;

export function TransitionGraph({ stops, maxNodes = 9 }: Props) {
  const { nodes, edges } = deriveTransitionGraph(stops, maxNodes);

  // A single node has no transition to draw — the timeline covers that
  // case, so render nothing rather than a lonely dot.
  if (nodes.length < 2) return null;

  const height = PAD_Y * 2 + (nodes.length - 1) * ROW_H;
  const maxVisits = Math.max(...nodes.map((n) => n.visits), 1);
  const maxEdge = Math.max(...edges.map((e) => e.count), 1);
  const arcBand = VIEW_W - NODE_X - 8;
  const y = (i: number) => PAD_Y + i * ROW_H;

  return (
    <figure style={{ margin: 0, display: 'flex', flexDirection: 'column', gap: 8 }}>
      <svg
        width="100%"
        viewBox={`0 0 ${VIEW_W} ${height}`}
        preserveAspectRatio="xMidYMid meet"
        role="img"
        aria-label={`Schematic transition graph of ${nodes.length} stops, ordered by first visit. Arc thickness shows how often you travelled between two stops.`}
        style={{ display: 'block', maxWidth: '100%', overflow: 'visible' }}
      >
        {/* Arcs first so nodes + labels sit on top. */}
        <g fill="none" stroke="var(--accent)">
          {edges.map((e) => {
            const yA = y(e.a);
            const yB = y(e.b);
            const span = Math.abs(e.b - e.a);
            const bow = Math.min(arcBand, 18 + span * 16);
            const ctrlX = NODE_X + bow;
            const midY = (yA + yB) / 2;
            const w = 1 + (e.count / maxEdge) * 3.5;
            const opacity = 0.28 + (e.count / maxEdge) * 0.5;
            return (
              <path
                key={`${e.a}-${e.b}`}
                d={`M ${NODE_X} ${yA} Q ${ctrlX} ${midY} ${NODE_X} ${yB}`}
                strokeWidth={w}
                strokeOpacity={opacity}
                strokeLinecap="round"
              />
            );
          })}
        </g>
        {/* Guide rail down the node column. */}
        <line
          x1={NODE_X}
          y1={y(0)}
          x2={NODE_X}
          y2={y(nodes.length - 1)}
          stroke="var(--border)"
          strokeWidth={1}
        />
        {nodes.map((n, i) => {
          const r = 3 + Math.sqrt(n.visits / maxVisits) * 5;
          const isTop = n.visits === maxVisits;
          return (
            <g key={n.key}>
              <text
                x={LABEL_W}
                y={y(i)}
                textAnchor="end"
                dominantBaseline="middle"
                fontSize={11}
                fill="var(--fg)"
              >
                {truncate(n.label, 22)}
                <tspan dx={6} fill="var(--fg-dim)" fontSize={10}>
                  {n.visits}
                  {'×'}
                </tspan>
              </text>
              <circle
                cx={NODE_X}
                cy={y(i)}
                r={r}
                fill={isTop ? 'var(--accent)' : 'var(--bg-elev)'}
                stroke="var(--accent)"
                strokeWidth={1.5}
              />
            </g>
          );
        })}
      </svg>
      <figcaption className="hud-note" style={{ margin: 0 }}>
        Schematic — stops ordered by first visit, not distance. Arc width =
        trips between two stops; dot size = visits.
      </figcaption>
    </figure>
  );
}

function truncate(s: string, max: number): string {
  return s.length > max ? s.slice(0, max - 1) + '…' : s;
}
