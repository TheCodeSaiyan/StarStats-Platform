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
  window.location.assign('/auth/logout');
}
