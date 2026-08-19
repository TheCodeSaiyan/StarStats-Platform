import { describe, it, expect } from 'vitest';

import { localInputToUtcIso, utcIsoToLocalInput } from './expiry';

// A spread of offsets covering both signs, zero, and the half-hour zones
// (India UTC+5:30 → -330). `getTimezoneOffset()` returns UTC−local in minutes,
// so a zone AHEAD of UTC is NEGATIVE (UTC+2 → -120) and BEHIND is positive.
const OFFSETS = [-120, 0, 300, -330, 600, -720];

describe('localInputToUtcIso', () => {
  it('converts a naive local wall-clock to the correct UTC instant (UTC+2)', () => {
    // 10:00 in UTC+2 is 08:00Z.
    expect(localInputToUtcIso('2026-06-01T10:00', -120)).toBe(
      '2026-06-01T08:00:00.000Z',
    );
  });

  it('converts a naive local wall-clock to the correct UTC instant (UTC-5)', () => {
    // 10:00 in UTC-5 is 15:00Z.
    expect(localInputToUtcIso('2026-06-01T10:00', 300)).toBe(
      '2026-06-01T15:00:00.000Z',
    );
  });

  it('does NOT depend on the runtime timezone (offset 0 is identity)', () => {
    expect(localInputToUtcIso('2026-06-01T10:00', 0)).toBe(
      '2026-06-01T10:00:00.000Z',
    );
  });

  it('handles half-hour zones (India UTC+5:30 → offset -330)', () => {
    // 10:00 in UTC+5:30 is 04:30Z.
    expect(localInputToUtcIso('2026-06-01T10:00', -330)).toBe(
      '2026-06-01T04:30:00.000Z',
    );
  });

  it('returns null for empty or unparseable input', () => {
    expect(localInputToUtcIso('', -120)).toBeNull();
    expect(localInputToUtcIso('   ', -120)).toBeNull();
    expect(localInputToUtcIso('not-a-date', -120)).toBeNull();
  });
});

describe('utcIsoToLocalInput', () => {
  it('converts a UTC instant back to the local wall-clock (UTC+2)', () => {
    expect(utcIsoToLocalInput('2026-06-01T08:00:00.000Z', -120)).toBe(
      '2026-06-01T10:00',
    );
  });

  it('returns empty string for an unparseable ISO', () => {
    expect(utcIsoToLocalInput('nonsense', -120)).toBe('');
  });
});

describe('round-trip fixed point', () => {
  it('local → utc → local is a fixed point across offsets', () => {
    const local = '2026-06-01T10:00';
    for (const offset of OFFSETS) {
      const iso = localInputToUtcIso(local, offset);
      expect(iso).not.toBeNull();
      expect(utcIsoToLocalInput(iso as string, offset)).toBe(local);
    }
  });

  it('utc → local → utc is a fixed point across offsets', () => {
    const iso = '2026-06-01T08:00:00.000Z';
    for (const offset of OFFSETS) {
      const local = utcIsoToLocalInput(iso, offset);
      expect(localInputToUtcIso(local, offset)).toBe(iso);
    }
  });
});
