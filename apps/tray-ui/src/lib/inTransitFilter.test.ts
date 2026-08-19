import { describe, it, expect } from 'vitest';

import { IN_TRANSIT_HIDDEN_TYPES, isInTransitNoise } from './inTransitFilter';

describe('IN_TRANSIT_HIDDEN_TYPES', () => {
  it('suppresses the five self-explanatory movement variants', () => {
    // Keep in sync with apps/web/src/lib/event-filter.ts.
    expect(IN_TRANSIT_HIDDEN_TYPES.has('join_pu')).toBe(true);
    expect(IN_TRANSIT_HIDDEN_TYPES.has('change_server')).toBe(true);
    expect(IN_TRANSIT_HIDDEN_TYPES.has('quantum_target_selected')).toBe(true);
    expect(IN_TRANSIT_HIDDEN_TYPES.has('seed_solar_system')).toBe(true);
    expect(IN_TRANSIT_HIDDEN_TYPES.has('resolve_spawn')).toBe(true);
    expect(IN_TRANSIT_HIDDEN_TYPES.size).toBe(5);
  });

  it('does not suppress meaningful outcome events', () => {
    expect(isInTransitNoise('player_death')).toBe(false);
    expect(isInTransitNoise('mission_end')).toBe(false);
    expect(isInTransitNoise('')).toBe(false);
  });
});
