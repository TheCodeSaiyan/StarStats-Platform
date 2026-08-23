'use client';

import React from 'react';
import {
  PaneSurface,
  type SurfaceSection,
  type PaneSurfaceProps,
} from '@/components/projection/PaneSurface';
import { DISCOVER_GROUPS } from './groups';

/**
 * `/discover` — the **Directory**, in the projection.
 *
 * COVERAGE marks `Directory.jsx` as inferred, so nothing here comes from the
 * kit: it is ported from the route, and the e2e contract the flat page carried
 * (`discover-page`, `discover-grid`, `discover-profile-card`, `data-handle`,
 * `discover-empty-state`, `discover-load-more`, and the
 * `/u/{handle}?source=discover` href) is preserved exactly. Those hooks are how
 * three specs assert the listing; changing them would look like a passing
 * rewrite of tests that had stopped testing anything.
 *
 * ONE GROUP, so `PaneSurface` hides the lens rail — a rail with a single lit
 * item reads as a control that does not work.
 *
 * PUBLIC RENDER PATH. The listing endpoint is unauthenticated by design (the
 * same data is reachable per-handle), and the flat page never asked for a
 * session. The chrome still needs one to know whether to offer the account
 * menu, so the session is read but never required.
 */
export type DiscoverSection = SurfaceSection;

export type DiscoverProjectionProps = Omit<
  PaneSurfaceProps,
  'crumb' | 'account' | 'groups' | 'themeAction'
>;

export function DiscoverProjection(props: DiscoverProjectionProps) {
  return (
    <PaneSurface
      {...props}
      // Static public surface — the directory is public, so it carries the CIG
      // trademark plate. See `PaneSurface`'s `legal` prop.
      legal
      groups={DISCOVER_GROUPS}
      crumb={
        props.handle
          ? [{ label: 'Projection', href: '/me' }, { label: 'Directory' }]
          : [{ label: 'Site', href: '/' }, { label: 'Directory' }]
      }
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
