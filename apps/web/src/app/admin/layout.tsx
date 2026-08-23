import { redirect } from 'next/navigation';
import { getSession } from '@/lib/session';
import { getTheme } from '@/lib/theme';
import { navSections } from '@/lib/nav';
import type { Calibration } from 'holo';
import { setCalibrationAction } from '@/app/me/_projection/actions';
import { ConsoleShell } from './_projection/ConsoleShell';

// Segment-level title (M-W12): every /admin/** page inherits this
// unless it exports its own metadata, so the whole admin surface shows
// "Admin — StarStats" instead of the bare "StarStats" default.
export const metadata = { title: 'Admin' };

/**
 * Server-component gate for the /admin surface, and its Console frame.
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
 *
 * PROJECTION PORT. The frame moved into `ConsoleShell`. The `role="main"`
 * landmark that used to live on this file's wrapper div now sits on
 * `Projection`'s `#hp-content`, which wraps the page body and excludes the
 * chrome; M-W9 still applies in that it is a DIV, since globals.css clamps a
 * bare `<main>` into a 720px column that would crush every admin table. Twenty pages inherit the Console chrome from
 * here without any of them changing; their own content renders through the
 * flat-primitive bridge until each is redrawn. `AdminNav` stays where it was
 * and is passed to the shell's lens slot — it used to be imported and rendered
 * by all 21 pages, each passing its own `current`, and that consolidation is
 * not being undone.
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

  let calibration: Calibration = 'terra';
  try {
    calibration = (await getTheme(session.token)) as Calibration;
  } catch {
    // Preference read failed; the default stands.
  }

  return (
    <ConsoleShell
      handle={session.claimedHandle}
      calibration={calibration}
      nav={navSections(
        { signedIn: true, staffRoles: session.staffRoles },
        'admin',
      )}
      onCalibrate={async (id: string) => {
        'use server';
        await setCalibrationAction(id);
      }}
    >
      {children}
    </ConsoleShell>
  );
}
