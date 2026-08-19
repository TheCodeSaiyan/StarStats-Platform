import React from 'react';
import { describe, expect, it } from 'vitest';
import { render, screen } from '@testing-library/react';
import { SyncChip, relativeTime, retentionWindow } from './user-activity';

describe('relativeTime', () => {
  const now = new Date('2026-08-11T12:00:00Z');

  // The bug this exists to prevent: a null timestamp coerced into a
  // Date renders as "just now" or "56 years ago" and looks like data.
  it('renders null as "never", not a date', () => {
    expect(relativeTime(null, now)).toBe('never');
    expect(relativeTime(undefined, now)).toBe('never');
  });

  it('does not confuse never with a genuine recent timestamp', () => {
    expect(relativeTime('2026-08-11T11:59:30Z', now)).toBe('just now');
    expect(relativeTime(null, now)).not.toBe('just now');
  });

  it('scales the unit with the gap', () => {
    expect(relativeTime('2026-08-11T11:30:00Z', now)).toBe('30m ago');
    expect(relativeTime('2026-08-11T06:00:00Z', now)).toBe('6h ago');
    expect(relativeTime('2026-08-02T12:00:00Z', now)).toBe('9d ago');
  });

  it('returns "unknown" for an unparseable value rather than NaN', () => {
    expect(relativeTime('not-a-date', now)).toBe('unknown');
  });
});

describe('retentionWindow', () => {
  // null means unlimited (the supporter tier). Rendering it as 0 would
  // claim a supporter's data is purged immediately.
  it('renders null as unlimited, not 0 days', () => {
    expect(retentionWindow(null)).toBe('unlimited');
    expect(retentionWindow(undefined)).toBe('unlimited');
  });

  it('renders a real window in days', () => {
    expect(retentionWindow(90)).toBe('90 days');
  });
});

describe('SyncChip', () => {
  // The failure mode is a UNIFORM column: if the devices join is
  // dropped every user reads "never", and any "does a chip render"
  // assertion still passes. These assert the states are distinct.
  it('renders a distinct label per state', () => {
    const { rerender } = render(<SyncChip state="live" />);
    expect(screen.getByText('Live')).toBeInTheDocument();

    rerender(<SyncChip state="stale" />);
    expect(screen.getByText('Stale')).toBeInTheDocument();

    rerender(<SyncChip state="off" />);
    expect(screen.getByText('Off')).toBeInTheDocument();

    rerender(<SyncChip state="never" />);
    expect(screen.getByText('Never')).toBeInTheDocument();
  });

  it('falls back to never for an unrecognised state', () => {
    render(<SyncChip state="wat" />);
    expect(screen.getByText('Never')).toBeInTheDocument();
  });

  it('explains the state in a title so the chip is not a bare word', () => {
    render(<SyncChip state="stale" />);
    expect(screen.getByText('Stale')).toHaveAttribute(
      'title',
      expect.stringContaining('7 days') as unknown as string,
    );
  });
});
