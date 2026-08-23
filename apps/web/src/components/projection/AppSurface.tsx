'use client';

import React from 'react';
import { PaneSurface, type PaneSurfaceProps } from './PaneSurface';

/**
 * The signed-in app pages that are not one of the named surfaces — `/orgs`,
 * `/contracts`, `/submissions` and their sub-routes.
 *
 * CHROME PORT, like the Console and the marketing set: each page keeps its own
 * body and renders through the flat-primitive bridge. These are the last three
 * places the flat `TopBar` was still live, so moving them is what lets the flat
 * shell be deleted — the bodies can be redrawn one at a time afterwards.
 *
 * `crumbHeading` is off: every one of these renders its own `<h1>`.
 */
export type AppSurfaceProps = Omit<
  PaneSurfaceProps,
  'groups' | 'account' | 'themeAction' | 'crumbHeading'
>;

export function AppSurface(props: AppSurfaceProps) {
  return (
    <PaneSurface
      {...props}
      groups={[{ key: 'page', label: 'Page' }]}
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
