'use client';

import React from 'react';
import { PaneSurface, type PaneSurfaceProps } from './PaneSurface';

/**
 * Client half of `MarketingSurface` — see the note there for why these pages
 * are chrome-only ports.
 *
 * One group, so `PaneSurface` hides the lens rail; `crumbHeading` is off
 * because each page brings its own `<h1>`.
 */
export type MarketingProjectionProps = Omit<
  PaneSurfaceProps,
  'groups' | 'account' | 'themeAction' | 'crumbHeading'
>;

export function MarketingProjection(props: MarketingProjectionProps) {
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
