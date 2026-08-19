/**
 * Tests for the parser-health page's run-staleness logic.
 *
 * This is the bit that decides whether "no findings" is reassuring or
 * meaningless. Getting it wrong reintroduces exactly the blind spot the
 * feature exists to remove, so it is tested directly rather than through
 * the rendered page.
 */
import React from 'react';
import { describe, expect, it } from 'vitest';
import { runStaleness } from './staleness';

const NOW = new Date('2026-08-07T12:00:00Z').getTime();

function run(over: Partial<Parameters<typeof runStaleness>[0] & object> = {}) {
  return {
    id: 1,
    started_at: '2026-08-07T11:00:00Z',
    finished_at: '2026-08-07T11:00:05Z',
    window_start: '2026-07-03T11:00:00Z',
    window_end: '2026-08-07T11:00:00Z',
    types_examined: 27,
    findings_open: 0,
    error: null,
    ...over,
  };
}

describe('runStaleness', () => {
  it('reports ok for a recent completed pass', () => {
    expect(runStaleness(run(), NOW).state).toBe('ok');
  });

  it('reports never when the detector has not run', () => {
    expect(runStaleness(null, NOW)).toEqual({ state: 'never', ageHours: null });
    expect(runStaleness(undefined, NOW).state).toBe('never');
  });

  it('reports failed when the pass recorded an error', () => {
    // A failed pass must not read as healthy just because it is recent.
    expect(
      runStaleness(run({ error: 'connection refused' }), NOW).state,
    ).toBe('failed');
  });

  it('reports stale once the last pass is older than the threshold', () => {
    // Daily cadence; 36h without a pass means the loop has stopped.
    const old = run({
      started_at: '2026-08-05T00:00:00Z',
      finished_at: '2026-08-05T00:00:05Z',
    });
    const s = runStaleness(old, NOW);
    expect(s.state).toBe('stale');
    expect(Math.round(s.ageHours ?? 0)).toBe(60);
  });

  it('reports stale for a pass that started but never finished', () => {
    // A crashed mid-pass leaves finished_at null; that is not "healthy".
    expect(runStaleness(run({ finished_at: null }), NOW).state).toBe('stale');
  });

  it('measures age from finished_at, falling back to started_at', () => {
    const s = runStaleness(run({ finished_at: null }), NOW);
    // started_at is 1h before NOW.
    expect(Math.round(s.ageHours ?? 0)).toBe(1);
  });
});
