import { commendLabel } from '@/lib/commends';

/**
 * One line of text for a notification, by kind.
 *
 * Kept apart from the page so it can be tested, and so a kind this build does
 * not know (a newer server) reads as something neutral. It used to fall
 * through to "accepted your friend request", which a salute would have been
 * shown as.
 */
export function notificationText(
  kind: string,
  actor: string | null | undefined,
  payload?: unknown,
): string {
  const who = `@${actor ?? 'Someone'}`;
  switch (kind) {
    case 'friend_request':
      return `${who} sent you a friend request`;
    case 'friend_accepted':
      return `${who} accepted your friend request`;
    case 'salute':
      return `${who} saluted your profile. o7`;
    case 'lfg_join_request':
      return `${who} asked to join your group`;
    case 'lfg_join_accepted':
      return `${who} accepted you into their group`;
    case 'commend': {
      // Anonymous by design: the payload names the word, never the giver.
      const word = (payload as { kind?: unknown } | null | undefined)?.kind;
      return typeof word === 'string'
        ? `A crewmate commended you: ${commendLabel(word)}`
        : 'A crewmate commended you';
    }
    default:
      return `New activity from ${who}`;
  }
}
