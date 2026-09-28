/**
 * The Matrix session a browser keeps between visits: the access token and
 * device ID from the JWT login, so the same device (and its encryption
 * keys, in IndexedDB) is reused rather than a new device per page load.
 *
 * It is tied to the StarStats account that made it. A stored session for
 * someone else (a shared computer, a different sign-in) is never used; it
 * is thrown away, and sign-out clears it (see `lib/sign-out.ts`).
 */
export const CHAT_SESSION_KEY = 'ss.chat.session';

export interface StoredChatSession {
  /** Lowercased StarStats handle the session belongs to. */
  handle: string;
  userId: string;
  deviceId: string;
  accessToken: string;
  homeserverUrl: string;
}

export function parseStoredSession(raw: string | null): StoredChatSession | null {
  if (!raw) return null;
  try {
    const v = JSON.parse(raw) as Partial<StoredChatSession>;
    if (
      typeof v.handle === 'string' &&
      typeof v.userId === 'string' &&
      typeof v.deviceId === 'string' &&
      typeof v.accessToken === 'string' &&
      typeof v.homeserverUrl === 'string'
    ) {
      return v as StoredChatSession;
    }
  } catch {
    // Corrupt: treat as absent.
  }
  return null;
}

/** The stored session, if it belongs to this StarStats account. */
export function sessionFor(raw: string | null, handle: string): StoredChatSession | null {
  const s = parseStoredSession(raw);
  return s && s.handle === handle.toLowerCase() ? s : null;
}

/** The IndexedDB name prefix for a user's encryption store. */
export function cryptoStorePrefix(userId: string): string {
  return `ss-chat-crypto:${userId}`;
}
