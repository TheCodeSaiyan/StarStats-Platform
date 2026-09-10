import { describe, it, expect } from 'vitest';
import {
  outstandingTasks,
  UPLINK_STALE_DAYS,
  type TaskInput,
} from './outstanding-tasks';
import type { DeviceDto } from './api';

const NOW = new Date('2026-09-10T12:00:00Z');

const device = (over: Partial<DeviceDto> = {}): DeviceDto =>
  ({
    id: 'dev-1',
    label: 'Gaming PC',
    created_at: '2026-09-01T12:00:00Z',
    last_seen_at: '2026-09-10T11:00:00Z',
    sync_enabled: true,
    ...over,
  }) as DeviceDto;

const input = (over: Partial<TaskInput> = {}): TaskInput => ({
  devices: [device()],
  eventTotal: 5000,
  rsiVerified: true,
  now: NOW,
  ...over,
});

const ids = (i: TaskInput) => outstandingTasks(i).map((t) => t.id);

describe('outstandingTasks — the data path', () => {
  it('says nothing when everything is working', () => {
    expect(ids(input())).toEqual([]);
  });

  it('asks for an uplink when none is paired', () => {
    expect(ids(input({ devices: [] }))).toEqual(['pair-uplink']);
  });

  it('asks for sync when paired but switched off', () => {
    expect(
      ids(input({ devices: [device({ sync_enabled: false })] })),
    ).toEqual(['enable-sync']);
  });

  // The case the old modal missed entirely: it fired on `total === 0`, so an
  // account with events whose uplink had since stopped saw nothing at all —
  // and a stopped uplink looks exactly like a quiet week.
  it('flags an uplink that has gone quiet', () => {
    const old = new Date(
      NOW.getTime() - (UPLINK_STALE_DAYS + 1) * 24 * 60 * 60 * 1000,
    ).toISOString();
    expect(ids(input({ devices: [device({ last_seen_at: old })] }))).toEqual([
      'uplink-quiet',
    ]);
  });

  it('does not nag about a quiet weekend', () => {
    const recent = new Date(
      NOW.getTime() - (UPLINK_STALE_DAYS - 1) * 24 * 60 * 60 * 1000,
    ).toISOString();
    expect(ids(input({ devices: [device({ last_seen_at: recent })] }))).toEqual(
      [],
    );
  });

  it('tells a syncing-but-silent uplink to check its folder', () => {
    expect(ids(input({ eventTotal: 0 }))).toEqual(['point-at-log']);
  });

  /**
   * The pipeline rule. Every stage below is unmet at once — no device, so
   * nothing syncing, so no events — and reporting all three would be both
   * noise and nonsense: there is nothing to enable sync ON.
   */
  it('reports only the earliest unmet stage, never the whole pipeline', () => {
    expect(ids(input({ devices: [], eventTotal: 0 }))).toEqual(['pair-uplink']);
  });

  it('counts a syncing uplink even when a second is switched off', () => {
    expect(
      ids(
        input({
          devices: [device({ sync_enabled: false }), device({ id: 'dev-2' })],
        }),
      ),
    ).toEqual([]);
  });

  // A failed summary call must not be read as "zero events" — that would
  // tell someone with a working uplink to go and fix their folder.
  it('stays quiet about the log when the event count is unknown', () => {
    expect(ids(input({ eventTotal: null }))).toEqual([]);
  });

  it('ignores an uplink that has never reported rather than calling it quiet', () => {
    // A just-paired device with no `last_seen_at` is not stale, it is new.
    expect(
      ids(input({ devices: [device({ last_seen_at: null })] })),
    ).toEqual([]);
  });
});

describe('outstandingTasks — account blockers', () => {
  it('flags an unverified RSI handle', () => {
    expect(ids(input({ rsiVerified: false }))).toEqual(['verify-rsi']);
  });

  /**
   * Account blockers are not part of the data pipeline, so they stack with
   * it rather than replacing it. Someone can perfectly well have no uplink
   * AND an unverified handle, and fixing one does not fix the other.
   */
  it('stacks with a data-path task', () => {
    expect(ids(input({ devices: [], rsiVerified: false }))).toEqual([
      'pair-uplink',
      'verify-rsi',
    ]);
  });

  it('leads with the data path, because nothing else matters without data', () => {
    const tasks = outstandingTasks(input({ devices: [], rsiVerified: false }));
    expect(tasks[0].id).toBe('pair-uplink');
  });
});
