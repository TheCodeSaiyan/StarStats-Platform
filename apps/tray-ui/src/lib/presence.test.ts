import { describe, expect, it } from 'vitest';
import { presenceLabel } from './presence';

const p = (state: string | null, system: string | null = null) => ({
  handle: 'Alice',
  state,
  system,
  updated_at: null,
});

describe('presenceLabel', () => {
  it('reads each state', () => {
    expect(presenceLabel(p('online'))).toBe('Online');
    expect(presenceLabel(p('in_game'))).toBe('In game');
    expect(presenceLabel(p('in_quantum', 'Pyro'))).toBe('In quantum · Pyro');
    expect(presenceLabel(p('in_game', 'Stanton'))).toBe('In game · Stanton');
  });

  it('shows nothing for offline, unknown, or a state it does not know', () => {
    expect(presenceLabel(p(null))).toBeNull();
    expect(presenceLabel(undefined)).toBeNull();
    expect(presenceLabel(p('docked'))).toBeNull();
  });
});
