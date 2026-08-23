'use client';

import React from 'react';
import Link from 'next/link';
import type { Route } from 'next';
import { RangeTabs } from 'holo';
import {
  PaneSurface,
  type SurfaceSection,
  type SurfaceGroup,
  type PaneSurfaceProps,
} from '@/components/projection/PaneSurface';
import type { RangeId } from '@/lib/range';

/**
 * `/me/travel` — the standalone travel surface.
 *
 * COVERAGE lists this as partial: the kit has the Travel LENS inside
 * `Holotable.jsx` but not the page, and the two are not the same thing. The
 * lens is a bounded summary you glance at; this is the uncapped detail — every
 * route rather than the top six, the full system breakdown, the traffic
 * matrix. That difference is why the lens's Planes link here at all.
 *
 * WINDOWED, so the chrome carries `RangeTabs` — and like `/me`, the range is a
 * URL param (`?range=`) rather than client state, so the server components
 * re-query and the view stays shareable.
 */
export type TravelSection = SurfaceSection;

export const TRAVEL_GROUPS: readonly SurfaceGroup[] = [
  { key: 'routes', label: 'Routes' },
  { key: 'trail', label: 'Trail' },
];

export type TravelProjectionProps = Omit<
  PaneSurfaceProps,
  'crumb' | 'account' | 'groups' | 'themeAction' | 'chromeTrailing'
> & { range: RangeId };

export function TravelProjection({ range, ...props }: TravelProjectionProps) {
  return (
    <PaneSurface
      {...props}
      groups={TRAVEL_GROUPS}
      crumb={[{ label: 'Projection', href: '/me' }, { label: 'Travel' }]}
      account={[
        { id: 'me', label: 'Projection', href: '/me' },
        { id: 'sharing', label: 'Sharing', href: '/sharing' },
        { id: 'settings', label: 'Calibrate', href: '/settings' },
      ]}
      chromeTrailing={
        <RangeTabs
          active={range}
          renderItem={(id, label, isActive) => (
            <Link
              href={`/me/travel?range=${id}` as Route}
              // `aria-current`, not `aria-pressed`: a link to the current view,
              // not a toggle button.
              aria-current={isActive ? 'page' : undefined}
              scroll={false}
            >
              {label}
            </Link>
          )}
        />
      }
    />
  );
}
