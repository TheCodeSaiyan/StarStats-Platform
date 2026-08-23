import 'server-only';

import { getLocationTrace } from '@/lib/api';
import { getLocationCatalog } from '@/lib/reference';
import { rangeToHours, type RangeId } from '@/lib/range';
import { toDistinctStops } from '@/components/journey/trail-utils';
import { deriveTransitionGraph } from '@/components/journey/TransitionGraph';
import { layoutMapNodes, type MapLayout } from 'holo';
import { logger } from '@/lib/logger';

/**
 * The Travel lens's ring, in `map` mode.
 *
 * This element REPLACES the `journey` widget rather than sitting beside it.
 * Both are built from the same source — `GET /v1/me/location/trace` →
 * `toDistinctStops` → `deriveTransitionGraph` — so rendering the graph as a
 * ring AND as a route-map Plane would put one dataset on screen twice. The
 * product has already been bitten by exactly that shape once: `travel` was
 * swapped out of the default home layout because it and `routes` both showed
 * the same ranked destinations.
 *
 * `corridors` deliberately SURVIVES as a ranked Plane on the same lens: the
 * ring shows the shape of the travel, the list gives the counts, and neither
 * substitutes for the other.
 *
 * Nothing here is new data or a new endpoint. The only thing the projection
 * adds is polar placement, which lives in `holo/ring-layout`.
 */
export async function buildRingMap(
  token: string,
  range: RangeId,
): Promise<MapLayout> {
  const empty: MapLayout = { nodes: [], links: [], ticks: [] };

  let entries;
  try {
    const trace = await getLocationTrace(token, rangeToHours(range));
    entries = trace.entries ?? [];
  } catch (err) {
    logger.warn({ err, call: 'projection.ringmap' }, 'trace fetch failed');
    return empty;
  }
  if (entries.length === 0) return empty;

  const stops = toDistinctStops(entries);
  const { nodes, edges } = deriveTransitionGraph(stops);
  if (nodes.length === 0) return empty;

  // The graph's nodes carry key/label/visits but not which system they are in,
  // and B3 groups the ring by system. The stop key is `system|planet|city`
  // (`locationKey`), so the system is its first field — read from the key
  // rather than re-deriving from the label, which may be a raw engine id.
  const systemOf = new Map<string, string | null>();
  for (const s of stops) {
    const system = s.system?.trim() || s.key.split('|')[0]?.trim() || null;
    systemOf.set(s.key, system || null);
  }

  const mapStops = nodes.map((n) => ({
    name: n.label,
    system: systemOf.get(n.key) ?? null,
    visits: n.visits,
  }));

  const mapEdges = edges.map((e) => ({
    a: nodes[e.a].label,
    b: nodes[e.b].label,
    count: e.count,
  }));

  return layoutMapNodes(mapStops, mapEdges, await knownSystems());
}

/**
 * Every solar system the product's location catalogue knows about.
 *
 * Taken as the distinct non-null `system` field across catalogued locations
 * rather than by filtering on a `System` tier: the tier vocabulary is part of
 * the taxonomy-v2 rollout and not every row carries it, whereas `system` is the
 * field the catalogue has always indexed. Systems the reader has not visited
 * come back from `layoutMapNodes` as dim labelled ticks.
 */
async function knownSystems(): Promise<string[]> {
  try {
    const catalog = await getLocationCatalog();
    const seen = new Set<string>();
    for (const entry of catalog.byName.values()) {
      const s = entry.system?.trim();
      if (s) seen.add(s);
    }
    return [...seen].sort();
  } catch (err) {
    // A missing catalogue costs the unvisited-system ticks and nothing else —
    // the ring still draws every place the reader has actually been.
    logger.warn({ err, call: 'projection.knownSystems' }, 'catalog load failed');
    return [];
  }
}
