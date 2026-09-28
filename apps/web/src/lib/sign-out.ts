import { CHAT_SESSION_KEY, parseStoredSession } from '@/lib/chat/session';
import { forgetChatStorage } from '@/lib/chat/storage-key';

/**
 * Sign out from the chrome's account menu.
 *
 * `/auth/logout` is a route handler (clears the session cookie, 302 to `/`),
 * not a page, so this is a full navigation rather than a router push: the
 * cookie has to be gone before the next render, and a soft navigation would
 * reuse the signed-in layout. Every chrome surface passes this as
 * `onSignOut`; before that none did, and the button never rendered.
 */
export function signOut(): void {
  forgetChat();
  window.location.assign('/auth/logout');
}

/**
 * Chat keeps a device session and an encrypted store in this browser
 * (components/chat/ChatApp). Signing out of StarStats ends both: the
 * homeserver is asked to log the device out (keepalive, so the request
 * outlives the navigation; best-effort, since the store is forgotten either
 * way), and every chat key and store here is deleted.
 */
function forgetChat(): void {
  try {
    const stored = parseStoredSession(localStorage.getItem(CHAT_SESSION_KEY));
    if (stored) {
      void fetch(new URL('_matrix/client/v3/logout', stored.homeserverUrl), {
        method: 'POST',
        headers: { Authorization: `Bearer ${stored.accessToken}` },
        keepalive: true,
      }).catch(() => undefined);
    }
  } catch {
    // No storage: nothing was kept.
  }
  forgetChatStorage();
}
