import { describe, expect, it } from 'vitest';
import { activityLabel, regionLabel, timeLeft, voiceLabel } from './lfg';

describe('lfg labels', () => {
  it('names known values and makes unknown ones readable', () => {
    expect(activityLabel('bounty_hunting')).toBe('Bounty hunting');
    expect(activityLabel('fps')).toBe('FPS');
    expect(activityLabel('base_building')).toBe('Base building');
    expect(regionLabel('oce')).toBe('Oceania');
    expect(voiceLabel('required')).toBe('Voice required');
  });
});

describe('timeLeft', () => {
  const now = Date.parse('2026-09-27T12:00:00Z');
  it('reads hours and minutes', () => {
    expect(timeLeft('2026-09-27T13:20:00Z', now)).toBe('1 h 20 min left');
    expect(timeLeft('2026-09-27T14:00:00Z', now)).toBe('2 h left');
    expect(timeLeft('2026-09-27T12:05:30Z', now)).toBe('5 min left');
  });
  it('says ending in the last minute and after', () => {
    expect(timeLeft('2026-09-27T12:00:30Z', now)).toBe('ending');
    expect(timeLeft('2026-09-27T11:00:00Z', now)).toBe('ending');
  });
});
