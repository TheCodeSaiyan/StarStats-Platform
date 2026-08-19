import { describe, it, expect } from 'vitest';
import { toDistinctStops } from './trail-utils';
import type { TraceEntry } from '@/lib/api';

function entry(over: Partial<TraceEntry>): TraceEntry {
  return {
    planet: null,
    city: null,
    system: null,
    shard: null,
    source_event_type: 'planet_terrain_load',
    started_at: '2026-06-08T00:00:00Z',
    ended_at: '2026-06-08T00:00:00Z',
    event_count: 1,
    ...over,
  } as TraceEntry;
}

describe('toDistinctStops', () => {
  it('threads resolved_location into resolvedLabel/resolvedSlug', () => {
    const stops = toDistinctStops([
      entry({
        city: 'Outpost_col_m_frm_indy_001',
        system: 'Pyro',
        resolved_location: {
          display_name: 'Indy Farm Outpost',
          slug: 'indy-farm',
          tier: 'landmark',
          source: 'fuzzy',
        },
      }),
    ]);
    expect(stops).toHaveLength(1);
    expect(stops[0].resolvedLabel).toBe('Indy Farm Outpost');
    expect(stops[0].resolvedSlug).toBe('indy-farm');
    // Raw label is still retained as the fallback classKey / display.
    expect(stops[0].label).toBe('Outpost_col_m_frm_indy_001');
  });

  it('leaves resolved fields null when the entry has no resolved_location', () => {
    const stops = toDistinctStops([entry({ system: 'Stanton' })]);
    expect(stops[0].resolvedLabel).toBeNull();
    expect(stops[0].resolvedSlug).toBeNull();
  });
});
