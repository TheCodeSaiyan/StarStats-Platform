import { describe, it, expect } from 'vitest';
import { pickBucket, compareLabel, type CategoryStats } from './kb-stats';

describe('pickBucket', () => {
  const stats: CategoryStats = {
    groups: {
      combat: { 'speed.scm': { min: 200, p10: 205, p25: 210, p50: 222, p75: 240, p90: 270, max: 275, n: 84 } },
      __all__: { 'speed.scm': { min: 100, p10: 110, p25: 150, p50: 200, p75: 250, p90: 300, max: 350, n: 288 } },
    },
  };

  it('returns the entity peer-group bucket when present', () => {
    expect(pickBucket(stats, 'combat')['speed.scm'].n).toBe(84);
  });

  it('falls back to __all__ when the group is missing', () => {
    expect(pickBucket(stats, 'nonexistent')['speed.scm'].n).toBe(288);
  });

  it('returns empty object when neither present', () => {
    expect(pickBucket({ groups: {} }, 'combat')).toEqual({});
  });
});

describe('compareLabel', () => {
  it('labels vehicle families and __all__', () => {
    expect(compareLabel('vehicle', 'combat')).toBe('Combat ships');
    expect(compareLabel('vehicle', '__all__')).toBe('All vehicles');
  });

  it('de-slugs other-category keys', () => {
    expect(compareLabel('weapon', 'ballistic-cannon')).toBe('Ballistic cannon');
    expect(compareLabel('location', '__all__')).toBe('All locations');
  });
});
