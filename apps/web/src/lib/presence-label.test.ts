import { describe, expect, it } from 'vitest';
import { presenceLabel, type FriendPresence } from './presence-label';

const p = (state: FriendPresence['state'], system: string | null = null): FriendPresence => ({
  handle: 'Alice',
  state,
  system,
  updated_at: null,
});

describe('presenceLabel', () => {
  it('reads each state, with the system only when shared', () => {
    expect(presenceLabel(p('online'))).toBe('Online');
    expect(presenceLabel(p('in_game'))).toBe('In game');
    expect(presenceLabel(p('in_quantum', 'Pyro'))).toBe('In quantum · Pyro');
  });

  it('shows nothing for offline or unknown', () => {
    expect(presenceLabel(p(null))).toBeNull();
    expect(presenceLabel(undefined)).toBeNull();
  });
});
