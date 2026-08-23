'use client';

import React from 'react';
import {
  PaneSurface,
  type SurfaceSection,
  type SurfaceGroup,
  type PaneSurfaceProps,
} from '@/components/projection/PaneSurface';

/**
 * `/kb` — the catalogue, in the projection.
 *
 * PUBLIC. `/kb` and `/kb/[category]` are browsable without a session (the e2e
 * suite visits both with no login), so the chrome must render for a signed-out
 * visitor: `handle` is optional, the account menu becomes a Sign in action,
 * and the nav is filtered with `navFor({ signedIn })`.
 *
 * COVERAGE boundary, and it matters here: the kit's `Catalogue.jsx` was read
 * from `kb/[category]/page.tsx` for the BROWSE — substring search over display
 * and class name, the per-category facet key, sort with direction, pagination,
 * the facet-from-unfiltered-set rule. The entity DETAIL sheet was never read,
 * so nothing on `/kb/[category]/[slug]` comes from the kit; it is ported from
 * the route itself.
 */
export type KbSection = SurfaceSection;

export const KB_GROUPS: readonly SurfaceGroup[] = [{ key: 'kb', label: 'Catalogue' }];

export type KbProjectionProps = Omit<
  PaneSurfaceProps,
  'crumb' | 'account' | 'groups' | 'themeAction' | 'chromeTrailing'
> & { crumb: PaneSurfaceProps['crumb'] };

export function KbProjection({ crumb, ...props }: KbProjectionProps) {
  return (
    <PaneSurface
      {...props}
      // Static public surface — the catalogue is public, so it carries the CIG
      // trademark plate. See `PaneSurface`'s `legal` prop.
      legal
      groups={KB_GROUPS}
      crumb={crumb}
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
