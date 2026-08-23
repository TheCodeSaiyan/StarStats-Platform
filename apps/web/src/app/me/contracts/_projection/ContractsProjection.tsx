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
 * `/me/contracts` — contract history.
 *
 * One of the three screens COVERAGE marks as READ FROM SOURCE, and the kit got
 * it right: the `closed_by` outcome wording, the completed ÷ resolved
 * denominator, the 200-run cap that announces itself, and the per-step
 * objectives are all already how the real page behaves. So this is a redraw
 * rather than a redesign, and nothing here is inferred.
 *
 * "Contracts", never "missions". Engine ids (`mission_bounty_vhrt_01`) stay
 * verbatim wherever they surface — they are log literals.
 */
export type ContractsSection = SurfaceSection;

export const CONTRACTS_GROUPS: readonly SurfaceGroup[] = [
  { key: 'outcomes', label: 'Outcomes' },
  { key: 'runs', label: 'Runs' },
];

export type ContractsProjectionProps = Omit<
  PaneSurfaceProps,
  'crumb' | 'account' | 'groups' | 'themeAction' | 'chromeTrailing'
> & { range: RangeId };

export function ContractsProjection({
  range,
  ...props
}: ContractsProjectionProps) {
  return (
    <PaneSurface
      {...props}
      groups={CONTRACTS_GROUPS}
      crumb={[{ label: 'Projection', href: '/me' }, { label: 'Contracts' }]}
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
              href={`/me/contracts?range=${id}` as Route}
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
