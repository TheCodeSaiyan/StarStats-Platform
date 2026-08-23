import React from 'react';
import type { Calibration } from 'holo';
import { getSession } from '@/lib/session';
import { getTheme } from '@/lib/theme';
import { navSections } from '@/lib/nav';
import { setCalibrationAction } from '@/app/me/_projection/actions';
import { AppSurface } from './AppSurface';

/**
 * A whole route SEGMENT in the projection, framed from its `layout.tsx`.
 *
 * WHY A LAYOUT AND NOT A PAGE WRAP. The pages this frames — the org detail, the
 * submission detail, the profile's sessions and entities, the donate flow —
 * have between three and nine top-level `return`s each: not-found, denied,
 * empty, loading-ish and success branches. Wrapping every branch by hand is
 * dozens of edits on files whose whole point is that the branch you get depends
 * on data, and a missed one ships a page still wearing the old chrome. A layout
 * frames all of them at once, which is exactly why the admin Console was done
 * this way too.
 *
 * The trade is that the crumb and pane header are per-SECTION rather than
 * per-page. That is fine here: every one of these pages renders its own `<h1>`
 * naming the specific record, so `crumbHeading` stays off and the heading a
 * reader sees is still the precise one.
 *
 * A SERVER COMPONENT, so pages stay unaware of how the beam is persisted.
 */
export async function AppSectionSurface({
  crumb,
  title,
  ctx,
  children,
}: {
  crumb: { label: string; href?: string }[];
  /**
   * ReactNode rather than string, so a segment whose heading depends on the
   * ROUTE can pass a client component. `/auth/**` is the case: nine routes
   * share one layout and each is a different step of one flow, and a layout
   * cannot read the pathname.
   */
  title: React.ReactNode;
  ctx?: React.ReactNode;
  children: React.ReactNode;
}) {
  const session = await getSession();

  let calibration: Calibration = 'terra';
  try {
    calibration = (await getTheme(session?.token)) as Calibration;
  } catch {
    // Preference read failed; the default stands.
  }

  return (
    <AppSurface
      handle={session?.claimedHandle}
      calibration={calibration}
      nav={navSections({
        signedIn: Boolean(session),
        staffRoles: session?.staffRoles,
      })}
      crumb={crumb}
      sections={[
        {
          id: 'page',
          group: 'page',
          title,
          ctx,
          // The plate comes from `PaneSurface`, which carries it by default.
          // It used to be rendered here as well, which put TWO on every page
          // in this shell once the default flipped.
          node: <div className="hp-appsection">{children}</div>,
        },
      ]}
      notice={null}
      onCalibrate={async (id: string) => {
        'use server';
        await setCalibrationAction(id);
      }}
    />
  );
}
