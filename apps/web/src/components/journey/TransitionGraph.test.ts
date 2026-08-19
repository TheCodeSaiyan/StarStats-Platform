import { describe, it, expect } from 'vitest';
import { deriveTransitionGraph } from './TransitionGraph';
import type { DistinctStop } from './trail-utils';

function stop(key: string, label: string): DistinctStop {
  return {
    key,
    label,
    sublabel: null,
    system: null,
    planet: null,
    city: label,
    resolvedLabel: null,
    resolvedSlug: null,
    enteredAt: '2026-07-01T00:00:00Z',
    lastSeenAt: '2026-07-01T00:00:00Z',
    eventCount: 1,
  };
}

describe('deriveTransitionGraph', () => {
  it('returns empty graph for no stops', () => {
    expect(deriveTransitionGraph([])).toEqual({ nodes: [], edges: [] });
  });

  it('counts arrivals per distinct node and preserves first-visit order', () => {
    const stops = [stop('a', 'A'), stop('b', 'B'), stop('a', 'A')];
    const { nodes } = deriveTransitionGraph(stops);
    const byKey = Object.fromEntries(nodes.map((n) => [n.key, n]));
    expect(byKey.a.visits).toBe(2);
    expect(byKey.b.visits).toBe(1);
    // A first-visited before B -> lower order index.
    expect(byKey.a.order).toBeLessThan(byKey.b.order);
  });

  it('collapses A->B and B->A into one undirected weighted edge', () => {
    const stops = [stop('a', 'A'), stop('b', 'B'), stop('a', 'A')];
    const { nodes, edges } = deriveTransitionGraph(stops);
    expect(edges).toHaveLength(1);
    // Two trips between A and B (A->B then B->A).
    expect(edges[0].count).toBe(2);
    // Indices reference the ordered node array with a < b.
    expect(edges[0].a).toBeLessThan(edges[0].b);
    expect(nodes[edges[0].a]).toBeDefined();
    expect(nodes[edges[0].b]).toBeDefined();
  });

  it('caps the node set to the most-visited stops', () => {
    // 5 distinct nodes; a is visited most.
    const stops = [
      stop('a', 'A'),
      stop('b', 'B'),
      stop('a', 'A'),
      stop('c', 'C'),
      stop('a', 'A'),
      stop('d', 'D'),
      stop('e', 'E'),
    ];
    const { nodes } = deriveTransitionGraph(stops, 2);
    expect(nodes).toHaveLength(2);
    expect(nodes.some((n) => n.key === 'a')).toBe(true);
  });
});

// A `join_pu` (server hop) is in LOCATION_EVENT_TYPES so it reaches the
// trace, but `resolve_join_pu` sets planet/city/system ALL null on
// purpose — it carries shard info, not a place. `toDistinctStops` does
// not filter those, so each one became a stop labelled "In transit"
// (primaryLabel's last fallback) under the single key `'||'`.
//
// Measured on a real 24h window: 11 join_pu among 19 collapsed stops.
// Half the graph was phantom, and the busiest "corridor" came out as
// `In transit ⇄ microTech`. Over 7d the ratio is far smaller, which is
// why short windows looked worst — reported as "24h seems the worst".
//
// You did not visit a place called In transit. A server hop between two
// real stops must not break the corridor between them.
function nowhere(): DistinctStop {
  return {
    key: '||',
    label: 'In transit',
    sublabel: null,
    system: null,
    planet: null,
    city: null,
    resolvedLabel: null,
    resolvedSlug: null,
    enteredAt: '2026-07-01T00:00:00Z',
    lastSeenAt: '2026-07-01T00:00:00Z',
    eventCount: 1,
  };
}

describe('deriveTransitionGraph — place-less stops', () => {
  it('never makes a node out of a stop with no place', () => {
    const { nodes } = deriveTransitionGraph([stop('a', 'A'), nowhere(), stop('b', 'B')]);
    expect(nodes.map((n) => n.label)).not.toContain('In transit');
    expect(nodes.map((n) => n.key)).not.toContain('||');
  });

  it('bridges the two real stops either side of it', () => {
    // A -> (hop) -> B is one trip from A to B, not two trips via nowhere.
    const { nodes, edges } = deriveTransitionGraph([stop('a', 'A'), nowhere(), stop('b', 'B')]);
    expect(nodes).toHaveLength(2);
    expect(edges).toHaveLength(1);
    expect(edges[0].count).toBe(1);
  });

  it('does not invent a self-edge when the hop sits between two visits to ONE place', () => {
    // A -> (hop) -> A is standing still through a server change. Removing
    // the hop makes the two A entries adjacent, so they must re-collapse;
    // otherwise the graph gains an A⇄A edge and a phantom second visit.
    const { nodes, edges } = deriveTransitionGraph([stop('a', 'A'), nowhere(), stop('a', 'A')]);
    expect(edges).toHaveLength(0);
    expect(nodes).toHaveLength(1);
    expect(nodes[0].visits).toBe(1);
  });

  it('returns an empty graph when every stop is place-less', () => {
    expect(deriveTransitionGraph([nowhere(), nowhere()])).toEqual({ nodes: [], edges: [] });
  });
});
