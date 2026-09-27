import type { FriendPresence } from '../api';

/**
 * One friend's presence as a short label for their row, or null for
 * offline. Offline also covers "not sharing": the server makes the two
 * look the same, and so does this.
 */
export function presenceLabel(p: FriendPresence | undefined): string | null {
  if (!p || !p.state) return null;
  const where = p.system ? ` · ${p.system}` : '';
  switch (p.state) {
    case 'online':
      return 'Online';
    case 'in_game':
      return `In game${where}`;
    case 'in_quantum':
      return `In quantum${where}`;
    default:
      return null;
  }
}
