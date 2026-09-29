/**
 * Where a sign-in lands. `/me` unless the sign-in asked for one of the
 * pages in this list, which today is only `/chat`: the tray's chat window
 * signs in through a magic link and must land in chat.
 *
 * An allowlist, not a path check: an open `next` would let a crafted
 * sign-in link send someone anywhere once they are signed in.
 */
const ALLOWED = ['/chat'] as const;

export type SignInDestination = '/me' | (typeof ALLOWED)[number];

export function signInDestination(next: unknown): SignInDestination {
  return (ALLOWED as readonly unknown[]).includes(next) ? (next as SignInDestination) : '/me';
}
