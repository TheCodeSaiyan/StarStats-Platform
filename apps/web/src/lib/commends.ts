import type { CommendKind } from '@/lib/api';

/**
 * Crew commends: the closed list of words a crewmate can give, in the order
 * the server returns totals. Mirrored in the tray's `lib/commends.ts`.
 */
export const COMMEND_KINDS: readonly CommendKind[] = [
  'great_pilot',
  'good_comms',
  'reliable',
  'good_teacher',
];

const LABELS: Record<CommendKind, string> = {
  great_pilot: 'Great pilot',
  good_comms: 'Good comms',
  reliable: 'Reliable',
  good_teacher: 'Good teacher',
};

/** A kind's label; an unknown one (a newer server) reads as itself. */
export function commendLabel(kind: string): string {
  return (LABELS as Record<string, string>)[kind] ?? kind.replace(/_/g, ' ');
}
