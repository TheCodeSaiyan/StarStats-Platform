'use client';

import React from 'react';
import {
  PaneSurface,
  type PaneSurfaceProps,
} from '@/components/projection/PaneSurface';

/**
 * `/u/[handle]` — a reader's public profile, in the projection.
 *
 * THE REFUSED BRANCH ONLY. A visible profile is a volume now and renders
 * through `PublicProjection`; this frames "not available", which is two
 * sentences and no data — a static surface is the right shape for it, and it
 * is the branch a stranger is most likely to land on.
 *
 * (This file used to frame the visible profile too, with a note claiming the
 * blocker was that "the `/me` projection's element catalogue is owner-scoped".
 * That was wrong: `GET /v1/public/{handle}/share-scopes` is unauthenticated
 * and the page was already calling it. See `PublicProjection`.)
 *
 * NO CRUMB HEADING. The page's own hero already carries an `<h1>` naming the
 * handle. A second one from the crumb would be the same two-h1 mistake the
 * Console and the marketing set avoid.
 *
 * PUBLIC, AND THE CHROME HAS TO KNOW. A visitor with no session gets the public
 * nav and a Sign in; a signed-in reader gets their own destinations. The one
 * thing that must NOT leak either way is the profile owner's identity into the
 * chrome — `handle` here is the VIEWER's, never the subject's. Passing the
 * subject's handle would tell a stranger they were signed in as someone else.
 */
export type ProfileProjectionProps = Omit<
  PaneSurfaceProps,
  'groups' | 'account' | 'themeAction' | 'crumbHeading'
>;

export function ProfileProjection(props: ProfileProjectionProps) {
  return (
    <PaneSurface
      {...props}
      // Static public surface — a public profile is read by strangers, so it carries the CIG
      // trademark plate. See `PaneSurface`'s `legal` prop.
      legal
      groups={[{ key: 'profile', label: 'Profile' }]}
      crumbHeading={false}
      account={
        props.handle
          ? [
              { id: 'me', label: 'Projection', href: '/me' },
              { id: 'sharing', label: 'Sharing', href: '/sharing' },
              { id: 'settings', label: 'Calibrate', href: '/settings' },
            ]
          : undefined
      }
    />
  );
}
