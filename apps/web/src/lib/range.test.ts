import { describe, it, expect } from 'vitest';
import {
  parseRange,
  rangeToHours,
  rangeToDays,
  rangeToMetricsRange,
  rangeToSinceIso,
  rangeToWindowHours,
  rangeLabel,
} from './range';

describe('range helpers', () => {
  it('parseRange falls back to the 7d default for missing/unknown', () => {
    expect(parseRange(undefined)).toBe('7d');
    expect(parseRange('nonsense')).toBe('7d');
    expect(parseRange('7d')).toBe('7d');
    expect(parseRange('all')).toBe('all');
  });

  it('rangeToHours maps each id to its hour window', () => {
    expect(rangeToHours('24h')).toBe(24);
    expect(rangeToHours('7d')).toBe(24 * 7);
    expect(rangeToHours('all')).toBe(24 * 365);
  });

  it('rangeToDays maps each id to a day count', () => {
    expect(rangeToDays('24h')).toBe(1);
    expect(rangeToDays('7d')).toBe(7);
    expect(rangeToDays('30d')).toBe(30);
    expect(rangeToDays('90d')).toBe(90);
    expect(rangeToDays('all')).toBe(365);
  });

  it('rangeToMetricsRange passes every bucket through unchanged', () => {
    // This test previously asserted 24h -> 7d, encoding a server
    // limitation as intended behaviour. The endpoint now serves a 24h
    // bucket, so widening would be the bug.
    expect(rangeToMetricsRange('24h')).toBe('24h');
    expect(rangeToMetricsRange('7d')).toBe('7d');
    expect(rangeToMetricsRange('all')).toBe('all');
  });

  it('rangeToSinceIso returns now minus the range window', () => {
    const before = Date.now();
    const t = new Date(rangeToSinceIso('7d')).getTime();
    const delta = before - t;
    expect(delta).toBeGreaterThanOrEqual(168 * 3600_000 - 2000);
    expect(delta).toBeLessThanOrEqual(168 * 3600_000 + 2000);
  });
});

describe('every offered bucket is genuinely honoured', () => {
  it('"all" is 365 days — the retention limit, not unbounded', () => {
    // 365 days is the hard retention limit, so "everything we have" and
    // "the last year" are the same set. Sending `undefined` to mean
    // lifetime promised a depth the data does not have.
    expect(rangeToWindowHours('all')).toBe(24 * 365);
  });

  it('sends the exact window for every bucket', () => {
    expect(rangeToWindowHours('24h')).toBe(24);
    expect(rangeToWindowHours('7d')).toBe(24 * 7);
    expect(rangeToWindowHours('30d')).toBe(24 * 30);
    expect(rangeToWindowHours('90d')).toBe(24 * 90);
  });

  it('no longer widens a 24h pick to a week', () => {
    // This used to return '7d', so choosing "24h" rendered a WEEK under
    // a "24h" label. The server gained a 24h bucket rather than the
    // client hiding the gap.
    expect(rangeToMetricsRange('24h')).toBe('24h');
  });

  it('passes every other bucket through to the metrics endpoint unchanged', () => {
    for (const id of ['7d', '30d', '90d', 'all'] as const) {
      expect(rangeToMetricsRange(id)).toBe(id);
    }
  });

  it('labels the buckets by the window they actually cover', () => {
    expect(rangeLabel('24h')).toBe('last 24 hours');
    expect(rangeLabel('all')).toBe('last year');
  });
});
