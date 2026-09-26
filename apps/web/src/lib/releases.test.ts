import { describe, expect, it } from 'vitest';
import { pairByDay, type Release } from './releases';

// `notes` is an opaque object in the generated schema; tests pass real arrays.
const rel = (over: Omit<Partial<Release>, 'notes'> & { notes?: unknown }): Release =>
  ({
    id: over.tag ?? 'x',
    track: 'tray',
    tag: 'tray-v0.1.31',
    version: '0.1.31',
    channel: 'live',
    released_on: '2026-09-26',
    summary: '',
    notes: [],
    created_at: '2026-09-26T18:00:00Z',
    ...over,
  }) as Release;

const line = (text: string, surface: string) => ({ text, surfaces: [surface], prs: [134], roadmap: null });

describe('pairByDay', () => {
  it('pairs tray and platform releases from the same day into one entry', () => {
    const days = pairByDay([
      rel({
        tag: 'tray-v0.1.31',
        notes: [
          { kind: 'Fixed', lines: [line('Group the Review tab', 'Tray')] },
          { kind: 'New', lines: [line('Friends & notifications', 'Tray')] },
        ],
      }),
      rel({
        tag: 'v0.1.62',
        track: 'platform',
        version: '0.1.62',
        notes: [{ kind: 'New', lines: [line('Friends & notifications', 'Web')] }],
      }),
      rel({ tag: 'tray-v0.1.30', version: '0.1.30', released_on: '2026-09-21' }),
    ]);
    expect(days.map((d) => [d.date, d.tray, d.platform])).toEqual([
      ['2026-09-26', '0.1.31', '0.1.62'],
      ['2026-09-21', '0.1.30', null],
    ]);
    const today = days[0];
    expect(today.groups.map((g) => g.kind)).toEqual(['New', 'Fixed']);
    expect(today.groups[0].lines).toHaveLength(1);
    expect(today.groups[0].lines[0].surfaces).toEqual(['Tray', 'Web']);
  });

  it('shows the highest version when a track shipped twice in a day', () => {
    const days = pairByDay([
      rel({ tag: 'v0.1.59', track: 'platform', version: '0.1.59' }),
      rel({ tag: 'v0.1.61', track: 'platform', version: '0.1.61' }),
      rel({ tag: 'v0.1.60', track: 'platform', version: '0.1.60' }),
    ]);
    expect(days[0].platform).toBe('0.1.61');
  });

  it('keeps channels apart', () => {
    const days = pairByDay([
      rel({ tag: 'tray-v0.1.31-alpha.1', channel: 'alpha' }),
      rel({ tag: 'tray-v0.1.31' }),
    ]);
    expect(days).toHaveLength(2);
  });
});
