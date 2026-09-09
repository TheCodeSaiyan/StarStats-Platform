import React from 'react';
import { redirect } from 'next/navigation';
import { getSession } from '@/lib/session';
import { getMe } from '@/lib/api';
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
 * Runs before any /admin/** page renders, and asks `/v1/auth/me` who
 * the bearer token says this is. It deliberately does NOT read the
 * `staffRoles` mirror on the session cookie: that cookie is unsigned
 * JSON, and `httpOnly` stops page scripts from reading it, not the
 * person holding it from editing it. Anyone could set `r:["admin"]`
 * and draw the entire console. Roles on the token are signed by the
 * API, so they are the ones worth believing.
 *
 * That costs one round trip per /admin nav, which the cookie mirror
 * existed to avoid. It buys the gate back its meaning, and the mirror
 * still serves every non-admin surface where being wrong is cosmetic.
 *
 * The API endpoints under `/v1/admin/...` enforce the same check
 * server-side via `StaffRoleSet::has`, so this was never the only
 * thing standing between a forged cookie and admin data — it is the
 * thing standing between it and the admin UI.
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
  // Fail closed: a token the API won't vouch for gets no console.
  let staffRoles: string[] = [];
  let lookupFailed = false;
  try {
    staffRoles = (await getMe(session.token)).staff_roles ?? [];
  } catch {
    lookupFailed = true;
  }
  if (lookupFailed) {
    redirect('/auth/login?next=/admin');
  }
  const isStaff = staffRoles.some((r) => r === 'admin' || r === 'moderator');
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
        { signedIn: true, staffRoles },
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
