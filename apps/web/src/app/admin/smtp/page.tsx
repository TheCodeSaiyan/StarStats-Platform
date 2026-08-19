/**
 * SMTP config moved into /admin/settings#smtp (2026-08-11).
 *
 * A redirect, not a "moved here" card. A card rots into dead UI — the
 * /settings sharing stub outlived its purpose by several releases and
 * had to be deleted in v1.8.172. A redirect cannot.
 *
 * `redirect()` is NOT wrapped in try/catch: Next implements it by
 * throwing a NEXT_REDIRECT sentinel, so a catch here would swallow the
 * redirect and silently render nothing.
 */

import { redirect } from 'next/navigation';

export default function AdminSmtpRedirect() {
  redirect('/admin/settings#smtp');
}
