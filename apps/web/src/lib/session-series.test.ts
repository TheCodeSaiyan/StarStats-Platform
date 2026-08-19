import { describe, it, expect } from 'vitest';
import { lastNSessionDurationsMinutes } from './session-series';

/**
 * The sessions list arrives NEWEST-FIRST from the API (list[0] is the
 * most recent). The helper takes the most recent N sessions that have a
 * computable duration and returns them OLDEST-FIRST so the sparkline
 * reads left -> right through time.
 */
describe('lastNSessionDurationsMinutes', () => {
  it('returns durations in minutes, oldest-first, for the most recent 5', () => {
    // newest-first input: 60m, 30m, 90m, 20m, 45m, 10m (6 sessions)
    const sessions = [
      { started_at: '2026-01-06T00:00:00Z', ended_at: '2026-01-06T01:00:00Z' }, // 60
      { started_at: '2026-01-05T00:00:00Z', ended_at: '2026-01-05T00:30:00Z' }, // 30
      { started_at: '2026-01-04T00:00:00Z', ended_at: '2026-01-04T01:30:00Z' }, // 90
      { started_at: '2026-01-03T00:00:00Z', ended_at: '2026-01-03T00:20:00Z' }, // 20
      { started_at: '2026-01-02T00:00:00Z', ended_at: '2026-01-02T00:45:00Z' }, // 45
      { started_at: '2026-01-01T00:00:00Z', ended_at: '2026-01-01T00:10:00Z' }, // 10 (dropped, 6th)
    ];
    // most recent 5 = [60,30,90,20,45], reversed oldest-first = [45,20,90,30,60]
    expect(lastNSessionDurationsMinutes(sessions, 5)).toEqual([45, 20, 90, 30, 60]);
  });

  it('handles fewer than 5 sessions gracefully', () => {
    const sessions = [
      { started_at: '2026-01-02T00:00:00Z', ended_at: '2026-01-02T01:00:00Z' }, // 60
      { started_at: '2026-01-01T00:00:00Z', ended_at: '2026-01-01T00:30:00Z' }, // 30
    ];
    // oldest-first
    expect(lastNSessionDurationsMinutes(sessions)).toEqual([30, 60]);
  });

  it('skips open sessions (missing ended_at) and non-positive durations', () => {
    const sessions = [
      { started_at: '2026-01-03T00:00:00Z', ended_at: null }, // open -> skip
      { started_at: '2026-01-02T00:00:00Z', ended_at: '2026-01-02T01:00:00Z' }, // 60
      { started_at: '2026-01-01T02:00:00Z', ended_at: '2026-01-01T01:00:00Z' }, // negative -> skip
    ];
    expect(lastNSessionDurationsMinutes(sessions)).toEqual([60]);
  });

  it('returns an empty array when no session has a computable duration', () => {
    expect(
      lastNSessionDurationsMinutes([
        { started_at: '2026-01-01T00:00:00Z', ended_at: null },
        { started_at: undefined, ended_at: undefined },
      ]),
    ).toEqual([]);
  });

  it('returns an empty array for empty input', () => {
    expect(lastNSessionDurationsMinutes([])).toEqual([]);
  });

  it('ignores unparseable timestamps', () => {
    const sessions = [
      { started_at: 'not-a-date', ended_at: '2026-01-02T01:00:00Z' },
      { started_at: '2026-01-01T00:00:00Z', ended_at: '2026-01-01T00:30:00Z' }, // 30
    ];
    expect(lastNSessionDurationsMinutes(sessions)).toEqual([30]);
  });
});
