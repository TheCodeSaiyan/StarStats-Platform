/**
 * Polar layout for the ring's `map` mode (gap B3).
 *
 * Upstream, map nodes carried hand-authored angle + radius (`{ n: 'Orison',
 * a: -70, r: 26 }`) because the kit had a fixed cast of six places. A real
 * reader's stops come out of `getLocationTrace` → `toDistinctStops` →
 * `deriveTransitionGraph`, which already yields weighted nodes and undirected
 * A⇄B edges — everything the ring needs EXCEPT where to put them. That is what
 * this module supplies.
 *
 * Placement rule: angle is allocated per solar system, proportional to how many
 * of the reader's stops that system holds, so stops in one system sit adjacent
 * and an inter-system hop visibly crosses the ring. Systems the reader has
 * never visited become dim labelled ticks rather than empty arcs — dead space
 * that says "Pyro exists, you have not been" instead of reading as a fault.
 *
 * Pure functions, no React and no DOM, so the geometry is unit-testable.
 */

import type { RingNode, RingTick } from './components/Ring';

/** One distinct stop the reader has visited, before placement. */
export interface MapStop {
  /** Display name — also the value handed back by `onSelectNode`. */
  name: string;
  /** Solar system this stop belongs to, or null when unknown. */
  system: string | null;
  /** Visit weight (arrival count). Drives the dot radius. */
  visits: number;
}

/** An undirected corridor between two stops, by name. */
export interface MapEdge {
  a: string;
  b: string;
  count: number;
}

export interface MapLayout {
  nodes: RingNode[];
  links: [number, number, number][];
  ticks: RingTick[];
}

/**
 * The ring reads to about eight map nodes; past that it stops being a shape and
 * becomes an unreadable table. Callers should aggregate rather than raise this.
 */
export const MAX_MAP_NODES = 8;

/** Dot radius bounds, in the ring's 560-unit viewBox. */
const R_MIN = 7;
const R_MAX = 26;

/** Corridor stroke width bounds. */
const W_MIN = 1.2;
const W_MAX = 4;

/** Degrees. -90 puts the first system's arc at the top of the ring. */
const START_ANGLE = -90;

/**
 * Round before a number reaches an SVG attribute.
 *
 * Same reason as `Ring`'s `q`: an unrounded float serialised into markup can
 * disagree between the server render and the client one in its final digit and
 * trip a hydration mismatch. Angles and radii both end up as attributes.
 */
const q = (n: number): number => Math.round(n * 1000) / 1000;

/** Scale a value into [lo, hi] against a maximum, guarding a zero max. */
function scale(value: number, max: number, lo: number, hi: number): number {
  if (max <= 0) return lo;
  return q(lo + (Math.min(value, max) / max) * (hi - lo));
}

/**
 * Group stops by system and place them around the ring.
 *
 * `knownSystems` is the full set of systems the product knows about. Any of
 * those the reader has not visited comes back in `ticks`. The caller supplies
 * it — the web app reads the distinct `system` field across the KB location
 * catalogue.
 */
export function layoutMapNodes(
  stops: readonly MapStop[],
  edges: readonly MapEdge[],
  knownSystems: readonly string[] = [],
): MapLayout {
  if (stops.length === 0) {
    return { nodes: [], links: [], ticks: [] };
  }

  // Busiest stops first, then capped: the ring is a shape, not a table.
  const ranked = [...stops]
    .sort((x, y) => y.visits - x.visits || x.name.localeCompare(y.name))
    .slice(0, MAX_MAP_NODES);

  // Group by system, preserving the visit-rank order of first appearance so the
  // busiest system takes the first arc.
  const bySystem = new Map<string, MapStop[]>();
  for (const stop of ranked) {
    // An unknown system is its own bucket rather than being dropped — a stop
    // the taxonomy has not classified is still a place the reader has been.
    const key = stop.system ?? '';
    const bucket = bySystem.get(key);
    if (bucket) bucket.push(stop);
    else bySystem.set(key, [stop]);
  }

  const maxVisits = Math.max(...ranked.map((s) => s.visits), 0);
  const nodes: RingNode[] = [];

  // Arc width is proportional to how many stops the system holds, so a reader
  // who never leaves one system gets the whole ring for it (degrading to a
  // plain rank-ordered spread) and no empty arc can occur by construction.
  let cursor = START_ANGLE;
  for (const [system, group] of bySystem) {
    const arc = (group.length / ranked.length) * 360;
    group.forEach((stop, i) => {
      // Centre each stop in its own slice of the system's arc.
      const angle = q(cursor + (arc * (i + 0.5)) / group.length);
      nodes.push({
        n: stop.name,
        a: angle,
        r: scale(stop.visits, maxVisits, R_MIN, R_MAX),
        ctx: system || undefined,
      });
    });
    cursor += arc;
  }

  // Edges reference nodes by index, so only corridors whose BOTH ends survived
  // the node cap can be drawn. Dropping a half-attached edge is correct: a line
  // to a node that is not on the ring would point at nothing.
  const indexOf = new Map(nodes.map((n, i) => [n.n, i] as const));
  const maxCount = Math.max(...edges.map((e) => e.count), 0);
  const links: [number, number, number][] = [];
  for (const edge of edges) {
    const i = indexOf.get(edge.a);
    const j = indexOf.get(edge.b);
    if (i === undefined || j === undefined || i === j) continue;
    links.push([i, j, scale(edge.count, maxCount, W_MIN, W_MAX)]);
  }

  // Systems the reader has never reached. Spread across the ring's lower half,
  // clear of the busiest arc, which starts at the top.
  const visited = new Set(
    ranked.map((s) => s.system).filter((s): s is string => !!s),
  );
  // If NOT ONE stop carries a system, we know nothing about where this reader
  // has been at system level — and "unvisited" would then render every known
  // system as a place they have never gone, which is a confident falsehood
  // rather than a missing figure. Not knowing is not the same as not having
  // been, so say nothing.
  const unvisited =
    visited.size === 0 ? [] : knownSystems.filter((s) => !visited.has(s));
  const ticks: RingTick[] = unvisited.map((label, i) => ({
    label,
    a: q(90 - ((unvisited.length - 1) / 2 - i) * 18),
  }));

  return { nodes, links, ticks };
}

/**
 * Equal-weight lens segments (gap B1, degraded state).
 *
 * The ring's segment widths are meant to carry each lens's share of captured
 * events, which needs a per-event lens attribution the server does not send
 * yet. Until it does, segments are equal: a made-up proportion would be a
 * confidently wrong number, and this system's rule is to state the limit rather
 * than round it away. Swap for real shares when the attribution lands.
 */
export function equalShares<T extends { name: string }>(
  items: readonly T[],
): { name: string; share: number }[] {
  if (items.length === 0) return [];
  const share = 1 / items.length;
  return items.map((it) => ({ name: it.name, share }));
}

/**
 * Normalise weighted lens shares so they sum to 1, which is what `Ring` expects
 * of `segments`. Returns equal shares when every weight is zero — an empty
 * window should still draw a navigable ring.
 */
export function sharesFromWeights<T extends { name: string; weight: number }>(
  items: readonly T[],
): { name: string; share: number }[] {
  const total = items.reduce((sum, it) => sum + Math.max(0, it.weight), 0);
  if (total <= 0) return equalShares(items);
  return items.map((it) => ({
    name: it.name,
    share: Math.max(0, it.weight) / total,
  }));
}
