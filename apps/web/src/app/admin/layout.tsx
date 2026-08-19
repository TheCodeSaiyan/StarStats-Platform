import { redirect } from 'next/navigation';
import { getSession } from '@/lib/session';
import { AdminNav } from './_components/AdminNav';

// Segment-level title (M-W12): every /admin/** page inherits this
// unless it exports its own metadata, so the whole admin surface shows
// "Admin — StarStats" instead of the bare "StarStats" default.
export const metadata = { title: 'Admin' };

/**
 * Server-component gate for the /admin surface.
 *
 * Runs before any /admin/** page renders. Uses the `staffRoles` field
 * mirrored into the session cookie at sign-in time, so role checks
 * don't pay an extra `/v1/auth/me` round trip per nav.
 *
 * Note: this is UX gating only. The API endpoints under
 * `/v1/admin/...` enforce the same check server-side via
 * `StaffRoleSet::has`, so a tampered cookie can't escalate.
 *
 * Admin implies moderator on the server side, so we accept either.
 */
export default async function AdminLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  const session = await getSession();
  if (!session) {
    redirect('/auth/login?next=/admin');
  }
  const isStaff = session.staffRoles.some(
    (r) => r === 'admin' || r === 'moderator',
  );
  if (!isStaff) {
    redirect('/me');
  }
  // Single main landmark for the whole /admin surface (M-W9). Uses
  // role="main" over a <main> element so the global `main {}` 720px
  // legacy column (globals.css) doesn't clamp full-width admin tables;
  // the one page that shipped its own <main> (parser-submissions/[id])
  // was de-nested to a plain <div> to keep exactly one landmark.
  // The nav lives here rather than in each page (it used to be imported
  // and rendered by all 21 of them, each passing its own `current`).
  // The screen-enter wrapper moves up with it so every admin page keeps
  // the same entrance animation and column spacing it had before.
  return (
    <div role="main">
      <div
        className="ss-screen-enter"
        style={{ display: 'flex', flexDirection: 'column', gap: 20 }}
      >
        <AdminNav />
        {children}
      </div>
    </div>
  );
}
