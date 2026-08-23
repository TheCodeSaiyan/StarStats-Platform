import { describe, it, expect } from 'vitest';
import { bucketSeries } from './series';

describe('bucketSeries', () => {
  it('normalises to the 0–100 scale the ring draws against', () => {
    // `Ring` computes bar length as `(v / 100) * 72`. Raw counts would draw
    // bars far past the ring on a busy account and a flat stub on a quiet one,
    // and nothing would error — so the scale is asserted, not assumed.
    const out = bucketSeries([1, 2, 3, 4], 4);
    expect(Math.max(...out)).toBe(100);
    expect(Math.min(...out)).toBeGreaterThanOrEqual(0);
  });

  it('sums rather than samples when folding', () => {
    // A sampled series silently drops the days between samples and still looks
    // like a complete picture.
    const out = bucketSeries([10, 10, 0, 0], 2);
    expect(out).toEqual([100, 0]);
  });

  it('returns nothing for no data, and nothing for all-zero', () => {
    // "No data" and "no activity" are different claims; the caller switches the
    // ring's mode on the difference, so neither may become a row of zeros.
    expect(bucketSeries([], 24)).toEqual([]);
    expect(bucketSeries([0, 0, 0], 24)).toEqual([]);
  });

  it('passes a short series through, still normalised', () => {
    expect(bucketSeries([5, 10], 24)).toEqual([50, 100]);
  });
});
