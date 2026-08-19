import { describe, it, expect } from 'vitest';
import {
  computeTrend,
  formatTrend,
  previousWindowLabel,
  PCT_FLOOR,
} from './trend';

const fmt = (n: number) => n.toLocaleString();

describe('computeTrend', () => {
  it('reports an increase with both delta and percent', () => {
    const t = computeTrend(120, 100);
    expect(t).toEqual({ direction: 'up', delta: 20, pct: 20 });
  });

  it('reports a decrease with a signed delta', () => {
    const t = computeTrend(80, 100);
    expect(t.direction).toBe('down');
    expect(t.delta).toBe(-20);
    expect(t.pct).toBe(-20);
  });

  it('reports no change when the windows match', () => {
    expect(computeTrend(42, 42)).toEqual({
      direction: 'flat',
      delta: 0,
      pct: null,
    });
  });

  // The whole point of the `first` direction. "+100%" or "+40 vs 0" both
  // imply a baseline that does not exist.
  it('reports a debut rather than an infinite percentage', () => {
    const t = computeTrend(40, 0);
    expect(t.direction).toBe('first');
    expect(t.delta).toBe(40);
    expect(t.pct).toBeNull();
  });

  // Both windows empty is a real comparison ("still nothing"), not a
  // debut — a debut would claim activity that did not happen.
  it('treats two empty windows as flat, not a debut', () => {
    expect(computeTrend(0, 0)).toEqual({
      direction: 'flat',
      delta: 0,
      pct: null,
    });
  });

  // A percentage off a tiny base exaggerates: 1 -> 2 is "+1", not a
  // doubling of anything meaningful.
  it('suppresses the percentage below the floor', () => {
    const t = computeTrend(2, 1);
    expect(t.direction).toBe('up');
    expect(t.delta).toBe(1);
    expect(t.pct).toBeNull();
  });

  it('reports a percentage at the floor', () => {
    const t = computeTrend(11, PCT_FLOOR);
    expect(t.pct).toBe(10);
  });
});

describe('formatTrend', () => {
  it('renders direction in the text, not only the arrow', () => {
    const s = formatTrend(computeTrend(120, 100), '7d', fmt);
    expect(s).toContain('+20');
    expect(s).toContain('(+20%)');
    expect(s).toContain('vs prev 7d');
    // Strip the glyph: the sentence must still convey the direction, so
    // the meaning survives a screen reader or a missing glyph.
    expect(s.replace('▲', '').trim()).toMatch(/^\+20/);
  });

  it('renders a decrease with a minus, not a bare number', () => {
    const s = formatTrend(computeTrend(80, 100), '30d', fmt);
    expect(s).toContain('−20');
    expect(s).toContain('(−20%)');
    expect(s).toContain('vs prev 30d');
  });

  it('names the empty predecessor instead of inventing a ratio', () => {
    const s = formatTrend(computeTrend(40, 0), '7d', fmt);
    expect(s).toContain('none in the prev 7d');
    expect(s).not.toContain('%');
    expect(s).not.toContain('vs prev');
  });

  it('says no change rather than +0', () => {
    const s = formatTrend(computeTrend(42, 42), '7d', fmt);
    expect(s).toContain('no change');
    expect(s).not.toContain('+0');
  });

  it('omits the percentage when it was suppressed', () => {
    const s = formatTrend(computeTrend(2, 1), '7d', fmt);
    expect(s).toContain('+1');
    expect(s).not.toContain('%');
  });

  it('appends a unit to the magnitude when given', () => {
    const s = formatTrend(computeTrend(1500, 1000), '7d', fmt, 'aUEC');
    expect(s).toContain('+500 aUEC');
  });

  it('formats large magnitudes through the supplied formatter', () => {
    const s = formatTrend(computeTrend(2_400_000, 1_200_000), '90d', fmt);
    expect(s).toContain('+1,200,000');
  });
});

describe('previousWindowLabel', () => {
  it('uses the range selector label so tile copy matches the control', () => {
    expect(previousWindowLabel('7d')).toBe('7d');
    expect(previousWindowLabel('24h')).toBe('24h');
    expect(previousWindowLabel('90d')).toBe('90d');
  });
});
