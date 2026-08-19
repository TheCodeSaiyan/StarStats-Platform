/**
 * Appearance defaults moved into /admin/settings#appearance
 * (2026-08-11).
 *
 * A redirect, not a "moved here" card — see the note on
 * admin/smtp/page.tsx. `redirect()` must not be wrapped in try/catch:
 * it throws a NEXT_REDIRECT sentinel that a catch would swallow.
 */

import { redirect } from 'next/navigation';

export default function AdminAppearanceRedirect() {
  redirect('/admin/settings#appearance');
}
