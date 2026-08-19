import { describe, it, expect } from 'vitest';
import { buildStatRow, bandLabel } from './kb-viz';
import type { Quantiles } from './kb-stats';

const q: Quantiles = { min: 200, p10: 205, p25: 210, p50: 222, p75: 240, p90: 270, max: 275, n: 84 };

describe('bandLabel', () => {
  it('top decile', () => expect(bandLabel(272, q).tone).toBe('high'));
  it('bottom decile', () => expect(bandLabel(201, q).tone).toBe('low'));
  it('near median', () => expect(bandLabel(222, q).tone).toBe('mid'));
});

describe('buildStatRow', () => {
  it('positions value on the min->max track and labels the band', () => {
    const row = buildStatRow('SCM speed', 'm/s', 262, q, 'metric');
    expect(row.label).toBe('SCM speed');
    expect(row.valueText).toBe('262 m/s');
    expect(row.fillPct).toBeGreaterThan(70);
    expect(row.fillPct).toBeLessThanOrEqual(100);
    expect(row.medianPct).toBeCloseTo(((222 - 200) / (275 - 200)) * 100, 1);
    expect(row.band?.tone).toBe('high');
  });

  it('degrades to a context-free row when stats are missing (n<5 → undefined q)', () => {
    const row = buildStatRow('Hull HP', 'hp', 11900, undefined, 'metric');
    expect(row.valueText).toBe('11,900 hp');
    expect(row.fillPct).toBeUndefined();
    expect(row.band).toBeUndefined();
  });

  it('converts distance units to imperial', () => {
    const row = buildStatRow('Max speed', 'm/s', 1000, undefined, 'imperial');
    expect(row.valueText).toContain('ft/s');
    expect(row.valueText.startsWith('3,28')).toBe(true);
  });
});
