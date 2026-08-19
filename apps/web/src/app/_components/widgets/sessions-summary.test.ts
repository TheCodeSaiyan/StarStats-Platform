import { describe, it, expect } from 'vitest';
import { buildSessionSummary } from './sessions-summary';

describe('buildSessionSummary', () => {
  it('uses the lifetime aggregate when present (owner view)', () => {
    const s = buildSessionSummary({
      lifetime: { session_count: 128, total_playtime_secs: 540_000 }, // 150h
      listLength: 50,
      derivedTotalMs: 47 * 3_600_000,
      listCap: 50,
    });
    expect(s.countLabel).toBe('128 sessions');
    expect(s.totalHoursLabel).toBe('150h played');
  });

  it('marks the count as capped (50+) when no lifetime and the list hit the cap', () => {
    const s = buildSessionSummary({
      lifetime: null,
      listLength: 50,
      derivedTotalMs: 47 * 3_600_000,
      listCap: 50,
    });
    expect(s.countLabel).toBe('50+ sessions');
    expect(s.totalHoursLabel).toBe('47h played');
  });

  it('uses the exact list count when below the cap and no lifetime', () => {
    const s = buildSessionSummary({
      lifetime: null,
      listLength: 3,
      derivedTotalMs: 0,
      listCap: 50,
    });
    expect(s.countLabel).toBe('3 sessions');
    expect(s.totalHoursLabel).toBeNull();
  });

  it('singularises a single session', () => {
    const s = buildSessionSummary({
      lifetime: { session_count: 1, total_playtime_secs: 1_800 },
      listLength: 1,
      derivedTotalMs: 1_800_000,
      listCap: 50,
    });
    expect(s.countLabel).toBe('1 session');
  });
});
