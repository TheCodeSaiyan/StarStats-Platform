/**
 * One line of text for a notification, by kind.
 *
 * Kept apart from the page so it can be tested, and so a kind this build does
 * not know (a newer server) reads as something neutral. It used to fall
 * through to "accepted your friend request", which a salute would have been
 * shown as.
 */
export function notificationText(kind: string, actor: string | null | undefined): string {
  const who = `@${actor ?? 'Someone'}`;
  switch (kind) {
    case 'friend_request':
      return `${who} sent you a friend request`;
    case 'friend_accepted':
      return `${who} accepted your friend request`;
    case 'salute':
      return `${who} saluted your profile. o7`;
    default:
      return `New activity from ${who}`;
  }
}
