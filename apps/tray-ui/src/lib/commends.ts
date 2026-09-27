import type { CommendKind, CrewOverview } from '../api';

/**
 * Crew commends: the closed list of words a crewmate can give. Mirrors the
 * web's `lib/commends.ts`; the server refuses anything else.
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

/**
 * Crewmates you could still commend: in an open window, and not commended
 * yet. What the Crew badge adds to the players waiting on your post.
 */
export function uncommended(crew: CrewOverview | null): number {
  if (!crew) return 0;
  return crew.windows.reduce((n, w) => n + w.crew.filter((m) => !m.my_commend).length, 0);
}
