import { describe, it, expect } from 'vitest';
import { PROJECTION_CATALOGUE } from './catalogue';
import { REGISTERED_IDS } from '@/app/_components/widgets/registry';
import { widgetMatchesLens } from '@/lib/lens';
import { RAIL_LENSES } from './rail';

/**
 * Feature-parity guards for the /me projection.
 *
 * The port dropped six widgets silently — `orgs`, `entities`, `records`,
 * `recent_activity`, `hangar` and `loadout` were all enable-able on the flat
 * dashboard and had no home in the projection at all, so a reader who had
 * turned any of them on simply lost it. Nothing failed; the screen just had
 * less in it.
 *
 * These are cheap, total checks over data rather than rendering, so a widget
 * added to the registry later cannot quietly skip the projection either.
 */
describe('projection catalogue covers the widget registry', () => {
  it('has an element for every registered widget', () => {
    const inCatalogue = new Set(PROJECTION_CATALOGUE.map((e) => e.id));
    const missing = REGISTERED_IDS.filter((id) => !inCatalogue.has(id));
    expect(missing).toEqual([]);
  });

  it('does not invent elements the registry does not have', () => {
    const registered = new Set<string>(REGISTERED_IDS);
    const extra = PROJECTION_CATALOGUE.map((e) => e.id).filter(
      (id) => !registered.has(id),
    );
    expect(extra).toEqual([]);
  });
});

describe('every element is reachable from some lens', () => {
  it('each pane element matches at least one lens in the rail', () => {
    // The rail IS `LENSES`, All included. Dropping All stranded every element
    // whose `WIDGET_LENSES` entry is the empty list (`records`, `orgs`,
    // `entities`, `facts`) — including `facts`, which is on by default and so
    // rendered nowhere at all.
    const railIds = RAIL_LENSES.map((l) => l.id);
    const unreachable = PROJECTION_CATALOGUE.filter(
      (e) => e.kind === 'plane' && !railIds.some((l) => widgetMatchesLens(e.id, l)),
    ).map((e) => e.id);
    expect(unreachable).toEqual([]);
  });

  it('keeps All in the rail — it is the only home for cross-cutting elements', () => {
    // Asserted on the RAIL, not on `LENSES`: the regression was a local
    // `.filter(l => l.id !== 'all')` inside the component, which a test
    // against `lib/lens.ts` cannot see.
    expect(RAIL_LENSES.map((l) => l.id)).toContain('all');
  });
});
