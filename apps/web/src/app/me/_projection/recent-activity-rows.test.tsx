import React from 'react';
import { describe, expect, it, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import { LogRow } from 'holo';

import { fmtLogTime, recentActivityRows } from './recent-activity-rows';

// next/link needs the App Router context; in jsdom an anchor is enough.
vi.mock('next/link', () => ({
  default: ({ href, children }: { href: string; children: React.ReactNode }) => (
    <a href={String(href)}>{children}</a>
  ),
}));

const NOW = new Date('2026-09-07T12:00:00');

function iso(local: string): string {
  return new Date(local).toISOString();
}

describe('fmtLogTime', () => {
  it('shows only the clock for an event from today', () => {
    const t = fmtLogTime(iso('2026-09-07T09:05:00'), NOW);
    expect(t.text).toMatch(/^\d{2}:\d{2}$/);
  });

  it('shows the date, not a misleading clock, for an older event', () => {
    // The live pane read "03:21 PM" for yesterday's events, indistinguishable
    // from today's. Older rows must carry the day instead.
    const t = fmtLogTime(iso('2026-09-06T15:21:00'), NOW);
    expect(t.text).toMatch(/^\d{1,2} [A-Z][a-z]{2}$/);
    expect(t.title).toBe(iso('2026-09-06T15:21:00'));
  });

  it('keeps the clock on an older event when asked, in a form that fits the column', () => {
    // Measured on the live page: "30 Sep 23:59" is 63px in the 64px cell, so
    // the month must stay three letters ("Sept" from the locale is 67px).
    const t = fmtLogTime(iso('2026-09-30T23:59:00'), new Date('2026-10-02T12:00:00'), { clock: true });
    expect(t.text).toBe('30 Sep 23:59');
    const today = fmtLogTime(iso('2026-09-07T09:05:00'), NOW, { clock: true });
    expect(today.text).toBe('09:05');
  });

  it('is the missing marker when there is no timestamp', () => {
    expect(fmtLogTime(null, NOW).text).toBe('—');
  });
});

describe('recentActivityRows', () => {
  const stowed = {
    seq: 1,
    event_type: 'vehicle_stowed',
    event_timestamp: iso('2026-09-07T10:00:00'),
    payload: {
      type: 'vehicle_stowed',
      timestamp: iso('2026-09-07T10:00:00'),
      vehicle_id: '816633546929',
      landing_area: 'LandingArea_ShipElevator_HangarMediumFront',
      landing_area_id: '1',
      zone_host_id: '2',
    },
  };

  it('renders a sentence from the payload, keeping the raw type in the tooltip', () => {
    const rows = recentActivityRows([stowed], undefined, { now: NOW });
    render(
      <>
        {rows.map((r) => (
          <LogRow key={r.key} time={r.time} event={r.event} tone={r.tone} mark={r.mark} />
        ))}
      </>,
    );
    expect(screen.getByText(/Stowed a ship at/)).toBeTruthy();
    expect(screen.queryByText('vehicle_stowed')).toBeNull();
    expect(screen.getByTitle('vehicle_stowed')).toBeTruthy();
  });

  it('falls back to the verb label, never the identifier, without a payload', () => {
    const rows = recentActivityRows(
      [
        {
          seq: 2,
          event_type: 'some_new_thing',
          event_timestamp: iso('2026-09-07T10:00:00'),
          payload: null,
        },
      ],
      undefined,
      { now: NOW },
    );
    render(
      <>
        {rows.map((r) => (
          <LogRow key={r.key} time={r.time} event={r.event} tone={r.tone} mark={r.mark} />
        ))}
      </>,
    );
    expect(screen.getByText('Some New Thing')).toBeTruthy();
    expect(screen.queryByText('some_new_thing')).toBeNull();
  });

  it('puts the event group in the mark column and tones combat rows', () => {
    const rows = recentActivityRows(
      [
        {
          seq: 3,
          event_type: 'actor_death',
          event_timestamp: iso('2026-09-07T10:00:00'),
          payload: {
            type: 'actor_death',
            timestamp: iso('2026-09-07T10:00:00'),
            victim: 'a',
            killer: 'b',
            weapon: 'w',
            damage_type: 'Bullet',
            zone: null,
          },
        },
      ],
      undefined,
      { now: NOW },
    );
    expect(rows[0].mark).toBe('Combat');
    expect(rows[0].tone).toBe('bad');
  });

  it('orders newest first by event time, not by ingest sequence', () => {
    // Deaths never fold, so both rows survive and only the order is under test.
    const death = {
      event_type: 'player_death',
      payload: { type: 'player_death', timestamp: stowed.event_timestamp, body_class: 'b', zone: null },
    };
    const older = { ...stowed, ...death, seq: 9, event_timestamp: iso('2026-09-07T09:00:00') };
    const newer = { ...stowed, ...death, seq: 4, event_timestamp: iso('2026-09-07T11:00:00') };
    const rows = recentActivityRows([older, newer], undefined, { now: NOW });
    expect(rows.map((r) => r.key)).toEqual(['4', '9']);
  });

  it('carries the clock into the rows when asked', () => {
    const rows = recentActivityRows(
      [{ ...stowed, event_timestamp: iso('2026-09-06T15:21:00') }],
      undefined,
      { now: NOW, clock: true },
    );
    render(<LogRow time={rows[0].time} event={rows[0].event} mark={rows[0].mark} />);
    expect(screen.getByText('6 Sep 15:21')).toBeTruthy();
  });

  it('caps the pane', () => {
    // Alternating types so nothing folds and the cap alone is under test.
    const death = {
      event_type: 'player_death',
      payload: { type: 'player_death', timestamp: stowed.event_timestamp, body_class: 'b', zone: null },
    };
    const many = Array.from({ length: 12 }, (_, i) =>
      i % 2 === 0 ? { ...stowed, seq: i } : { ...stowed, ...death, seq: i },
    );
    expect(recentActivityRows(many, undefined, { now: NOW, cap: 8 })).toHaveLength(8);
  });
});

describe('recentActivityRows folds runs', () => {
  const at = (local: string) => iso(local);
  const stow = (seq: number, local: string, vehicle_id: string) => ({
    seq,
    event_type: 'vehicle_stowed',
    event_timestamp: at(local),
    payload: {
      type: 'vehicle_stowed',
      timestamp: at(local),
      vehicle_id,
      landing_area: 'LandingArea_ShipElevator_HangarMediumFront',
      landing_area_id: '1',
      zone_host_id: '2',
    },
  });
  const death = (seq: number, local: string) => ({
    seq,
    event_type: 'player_death',
    event_timestamp: at(local),
    payload: { type: 'player_death', timestamp: at(local), body_class: 'body_01', zone: null },
  });

  function texts(rows: ReturnType<typeof recentActivityRows>): string[] {
    const { container } = render(
      <>
        {rows.map((r) => (
          <LogRow key={r.key} time={r.time} event={r.event} tone={r.tone} mark={r.mark} />
        ))}
      </>,
    );
    return Array.from(container.querySelectorAll('.ev')).map((el) => el.textContent ?? '');
  }

  it('collapses consecutive stows into one row with a count, anchored on the newest', () => {
    const rows = recentActivityRows(
      [stow(1, '2026-09-07T10:00:00', 'a'), stow(2, '2026-09-07T10:01:00', 'b'), stow(3, '2026-09-07T10:02:00', 'c')],
      undefined,
      { now: NOW },
    );
    expect(rows).toHaveLength(1);
    expect(rows[0].key).toBe('3');
    const [text] = texts(rows);
    // Same place three times, so the folded row keeps the sentence.
    expect(text).toMatch(/^Stowed a ship at .* ×3$/);
  });

  it('breaks a run on a different event', () => {
    const rows = recentActivityRows(
      [stow(1, '2026-09-07T10:00:00', 'a'), death(2, '2026-09-07T10:01:00'), stow(3, '2026-09-07T10:02:00', 'c')],
      undefined,
      { now: NOW },
    );
    expect(rows).toHaveLength(3);
  });

  it('never folds combat, even when identical', () => {
    const rows = recentActivityRows([death(1, '2026-09-07T10:00:00'), death(2, '2026-09-07T10:01:00')], undefined, {
      now: NOW,
    });
    expect(rows).toHaveLength(2);
  });

  it('caps after folding, so a long run does not eat the pane', () => {
    const many = Array.from({ length: 12 }, (_, i) => stow(i, `2026-09-07T10:${String(i).padStart(2, '0')}:00`, `v${i}`));
    const rows = recentActivityRows([...many, death(99, '2026-09-07T09:00:00')], undefined, { now: NOW, cap: 8 });
    expect(rows).toHaveLength(2);
  });
});
