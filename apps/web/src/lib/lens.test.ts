import { describe, it, expect } from 'vitest';
import { LENSES, WIDGET_LENSES, widgetMatchesLens } from './lens';
import { WIDGET_META } from '@/app/_components/widgets/widget-meta';
import type { WidgetId } from '@/app/_components/widgets/types';

describe('lens helpers', () => {
  it('All matches every widget', () => {
    expect(widgetMatchesLens('travel', 'all')).toBe(true);
    expect(widgetMatchesLens('orgs', 'all')).toBe(true);
    expect(widgetMatchesLens('anything-unknown', 'all')).toBe(true);
  });

  it('a dimension lens matches only its widgets', () => {
    expect(widgetMatchesLens('travel', 'travel')).toBe(true);
    expect(widgetMatchesLens('combat_mission', 'combat')).toBe(true);
    expect(widgetMatchesLens('contracts', 'combat')).toBe(true);
    expect(widgetMatchesLens('economy', 'commerce')).toBe(true);
    expect(widgetMatchesLens('heatmap', 'activity')).toBe(true);
    // non-members excluded
    expect(widgetMatchesLens('economy', 'combat')).toBe(false);
    expect(widgetMatchesLens('orgs', 'travel')).toBe(false);
  });

  it('LENSES leads with All', () => {
    expect(LENSES[0].id).toBe('all');
    expect(LENSES.map((l) => l.id)).toContain('combat');
  });
});

// `WIDGET_LENSES` was `Partial<Record<WidgetId, Lens[]>>`, and its own
// docstring admitted the consequence: "tsc won't catch a missing entry for
// a new widget id; adding one here is a manual step for every new widget."
// Seven of twenty-one widgets had drifted out, so picking any lens made them
// vanish — including `journey`, the route map, under the TRAVEL lens.
//
// Same defect as `ROWS_BY_ID`: a Partial map beside an exhaustive one, with
// a silent fallback instead of a compile error.
describe('WIDGET_LENSES covers the registry', () => {
  it('has an explicit lens list for every registered widget', () => {
    const ids = Object.keys(WIDGET_META) as WidgetId[];
    const missing = ids.filter((id) => WIDGET_LENSES[id] === undefined);
    expect(missing).toEqual([]);
  });

  // The four that were wrong, asserted by what their own docstrings say
  // they are — not by taste.
  it('puts the travel-derived widgets under the travel lens', () => {
    // "the owner's recent location trail rendered as a route map"
    expect(widgetMatchesLens('journey', 'travel')).toBe(true);
    // "top vehicle classes ranked by quantum-travel trip count"
    expect(widgetMatchesLens('fleet', 'travel')).toBe(true);
    // "hangar-vs-pad split ... of stow events"
    expect(widgetMatchesLens('docking', 'travel')).toBe(true);
  });

  it('puts life/death stats under the combat lens', () => {
    expect(widgetMatchesLens('lives', 'combat')).toBe(true);
  });

  // An empty list is a DECISION, not an oversight: these span every
  // dimension (or carry no data at all), so they belong only to All.
  it('keeps genuinely cross-cutting widgets out of every dimension lens', () => {
    for (const id of ['records', 'orgs', 'entities']) {
      expect(widgetMatchesLens(id, 'all')).toBe(true);
      for (const lens of ['travel', 'combat', 'commerce', 'activity', 'loadout'] as const) {
        expect(widgetMatchesLens(id, lens)).toBe(false);
      }
    }
  });
});
