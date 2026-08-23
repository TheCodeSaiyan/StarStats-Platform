'use client';

import React from 'react';
import {
  PaneSurface,
  type SurfaceSection,
  type SurfaceGroup,
  type PaneSurfaceProps,
} from '@/components/projection/PaneSurface';

/**
 * `/sharing` — the largest page in the app, in the projection.
 *
 * The kit's `Sharing.jsx` is inferred, not read (COVERAGE calls the
 * recreation "a sketch of it"), so nothing here comes from it: the five real
 * sections are ported from the route.
 *
 * Shell is `PaneSurface`, shared with `/settings` — including the fragment
 * wiring, which matters more here than anywhere: `#share-editor` is how the
 * edit flow works. Clicking Edit on a grant navigates to
 * `/sharing?edit=<handle>#share-editor`, and if the fragment did not select
 * the Outbound group the editor would not be mounted to scroll to.
 */
export type SharingSection = SurfaceSection;

export const SHARING_GROUPS: readonly SurfaceGroup[] = [
  { key: 'visibility', label: 'Visibility' },
  { key: 'outbound', label: 'Outbound' },
  { key: 'inbound', label: 'Inbound' },
  { key: 'views', label: 'Views' },
];

export type SharingProjectionProps = Omit<
  PaneSurfaceProps,
  'crumb' | 'account' | 'groups' | 'themeAction'
>;

export function SharingProjection(props: SharingProjectionProps) {
  return (
    <PaneSurface
      {...props}
      groups={SHARING_GROUPS}
      crumb={[{ label: 'Projection', href: '/me' }, { label: 'Sharing' }]}
      account={[
        { id: 'me', label: 'Projection', href: '/me' },
        { id: 'settings', label: 'Calibrate', href: '/settings' },
        { id: 'downloads', label: 'Emitter', href: '/downloads' },
      ]}
    />
  );
}
