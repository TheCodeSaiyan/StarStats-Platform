import React from 'react';
import type { Calibration } from 'holo';
import { getSession } from '@/lib/session';
import { getTheme } from '@/lib/theme';
import { navSections } from '@/lib/nav';
import { setCalibrationAction } from '@/app/me/_projection/actions';
import { MarketingProjection } from './MarketingProjection';

/**
 * The static prose surfaces — `/features`, `/about`, `/trust`, `/lore`,
 * `/docs`, `/guides`, `/privacy`, `/terms`, `/star-platform` — in the
 * projection.
 *
 * ONE SHELL, NINE PAGES, AND ONLY THE CHROME PORTED. Each page keeps its own
 * body; what changes is the frame around it. That is the same staging move as
 * the admin Console, for the same reason: nine pages of hand-written prose
 * layout is a body of work per page, and leaving them in the flat shell while
 * everything around them moved was the worse of the two states. Their content
 * renders through the flat-primitive bridge until each is redrawn, and the
 * bridge exists to be deleted a rule at a time as that happens.
 *
 * NO CRUMB HEADING. Every one of these pages already renders its own `<h1>`,
 * and it names the page better than a shared crumb could — so `crumbHeading` is
 * off and the page's own heading stands. Passing it would ship two h1s on all
 * nine at once.
 *
 * WORKS SIGNED IN AND SIGNED OUT. These routes are public but not
 * signed-out-only: a reader following a footer link to `/privacy` has a
 * session, and the flat shell wrapped them in `.ss-app` for exactly that case.
 * The chrome follows — account menu and the reader's own destinations when
 * there is a session, a Sign in and the public set when there is not.
 *
 * A SERVER COMPONENT. It reads the session and the calibration, so pages stay
 * `async` server components and none of them needs to know how the beam is
 * persisted.
 */
export async function MarketingSurface({
  /** Stable id from `SITE_NAV`, so the chrome marks the active destination. */
  navId,
  /** Trail back out. The last step is the page and is NOT a link. */
  crumb,
  /** Pane header. The page's own h1 sits inside the body, below it. */
  title,
  ctx,
  children,
}: {
  navId?: string;
  crumb: { label: string; href?: string }[];
  title: string;
  ctx?: string;
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
    <MarketingProjection
      handle={session?.claimedHandle}
      calibration={calibration}
      nav={navSections(
        { signedIn: Boolean(session), staffRoles: session?.staffRoles },
        navId,
      )}
      crumb={crumb}
      sections={[
        {
          id: 'page',
          title,
          ctx,
          group: 'page',
          // The plate comes from `PaneSurface`, which carries it by default.
          // It used to be rendered here as well, which put TWO on every page
          // in this shell once the default flipped.
          node: <div className="hp-marketing">{children}</div>,
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
