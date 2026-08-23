import { describe, it, expect } from 'vitest';
import { HOME_DEFAULT_LAYOUT, projectLayout } from './profile-layout';
import type { LayoutEntry } from './api';

describe('HOME_DEFAULT_LAYOUT', () => {
  it('enables the Me-tuned starter widgets', () => {
    const enabled = HOME_DEFAULT_LAYOUT.filter((e) => e.enabled).map((e) => e.id);
    // Curated home starts: activity heatmap, the lifetime character/fleet/
    // docking stat tiles, then the range-aware dimension widgets, plus the
    // mission-objectives and contract-run depth widgets (discoverable out
    // of the box), plus Player Facts (#368) — observations about how you
    // fly, distinct from `records`' superlatives.
    //
    // `hangar` joined when the "Hangar" NAV entry was removed. That entry
    // pointed at the paired-device page, not the fleet, and pairing moved into
    // Emitter — which left the actual RSI hangar with no default surface. It
    // is the owner's own page only; the public profile still ships it off.
    expect(enabled).toEqual([
      'heatmap',
      'lives',
      'fleet',
      'docking',
      'routes',
      'journey',
      'corridors',
      'facts',
      'combat_mission',
      'economy',
      'sessions',
      'hangar',
      'objectives',
      'contracts',
    ]);
  });

  // `travel` expanded already rendered "the top routes" and a link to
  // /journey — both of which are their own tiles here — so shipping it
  // beside `routes` would rank the same destinations twice on one screen.
  it('does not ship both travel and routes', () => {
    const enabled = HOME_DEFAULT_LAYOUT.filter((e) => e.enabled).map((e) => e.id);
    expect(enabled).toContain('routes');
    expect(enabled).not.toContain('travel');
  });

  // Dropping a widget from the DEFAULT must never remove it from the
  // registry projection — it stays in the editor's Add-widget palette.
  it('keeps travel available to re-enable', () => {
    expect(HOME_DEFAULT_LAYOUT.map((e) => e.id)).toContain('travel');
  });
});

// A widget added AFTER someone saved their layout was appended with a
// hardcoded `size: 'compact'`, discarding whatever the curated default
// said for it. `corridors` ships as `expanded` on /me because a bare
// corridor COUNT is not what the tile is for — but every existing owner
// received it compact, enabled it from the palette, and got the count.
// The intent has to survive the projection.
describe('projectLayout preserves the curated size for new widgets', () => {
  const registry = ['heatmap', 'corridors'] as const;

  it('takes an appended widget\'s size from the fallback, not a hardcoded compact', () => {
    const stored: LayoutEntry[] = [{ id: 'heatmap', enabled: true, size: 'expanded' }];
    const fallback: LayoutEntry[] = [
      { id: 'heatmap', enabled: true, size: 'expanded' },
      { id: 'corridors', enabled: true, size: 'expanded' },
    ];
    const out = projectLayout(stored, registry, fallback);
    const corridors = out.find((e) => e.id === 'corridors');
    expect(corridors?.size).toBe('expanded');
  });

  it('still appends it DISABLED — a saved layout must not gain widgets on its own', () => {
    const stored: LayoutEntry[] = [{ id: 'heatmap', enabled: true, size: 'expanded' }];
    const fallback: LayoutEntry[] = [
      { id: 'heatmap', enabled: true, size: 'expanded' },
      { id: 'corridors', enabled: true, size: 'expanded' },
    ];
    const out = projectLayout(stored, registry, fallback);
    expect(out.find((e) => e.id === 'corridors')?.enabled).toBe(false);
  });

  it('falls back to compact when the fallback layout does not mention it', () => {
    const stored: LayoutEntry[] = [{ id: 'heatmap', enabled: true, size: 'expanded' }];
    const fallback: LayoutEntry[] = [{ id: 'heatmap', enabled: true, size: 'expanded' }];
    const out = projectLayout(stored, registry, fallback);
    expect(out.find((e) => e.id === 'corridors')?.size).toBe('compact');
  });
});
