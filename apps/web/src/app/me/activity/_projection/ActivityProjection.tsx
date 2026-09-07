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
 * `/me/activity` — the full event log.
 *
 * The digest on `/me` shows eight rows; this is the uncapped page behind its
 * "see all". Same shell as `/me/contracts` and `/me/travel`: windowed, so the
 * chrome carries `RangeTabs`, and the range is a URL param rather than client
 * state so a filtered view stays shareable and back-button correct.
 */
export type ActivitySection = SurfaceSection;

export const ACTIVITY_GROUPS: readonly SurfaceGroup[] = [
  { key: 'log', label: 'Log' },
];

export type ActivityProjectionProps = Omit<
  PaneSurfaceProps,
  'crumb' | 'account' | 'groups' | 'themeAction' | 'chromeTrailing'
> & {
  range: RangeId;
  /** The other filters, already encoded, so a range change keeps them. */
  filterQuery: string;
};

export function ActivityProjection({
  range,
  filterQuery,
  ...props
}: ActivityProjectionProps) {
  return (
    <PaneSurface
      {...props}
      groups={ACTIVITY_GROUPS}
      crumb={[{ label: 'Projection', href: '/me' }, { label: 'Activity' }]}
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
              href={`/me/activity?range=${id}${filterQuery}` as Route}
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
