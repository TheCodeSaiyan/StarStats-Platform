/**
 * Ship Matrix config moved into /admin/settings#ship-matrix
 * (2026-08-11).
 *
 * A redirect, not a "moved here" card — see the note on
 * admin/smtp/page.tsx. `redirect()` must not be wrapped in try/catch:
 * it throws a NEXT_REDIRECT sentinel that a catch would swallow.
 */

import { redirect } from 'next/navigation';

export default function AdminShipMatrixRedirect() {
  redirect('/admin/settings#ship-matrix');
}
