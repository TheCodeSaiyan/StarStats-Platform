'use client';

import React from 'react';
import {
  PaneSurface,
  type SurfaceSection,
  type SurfaceGroup,
  type PaneSurfaceProps,
} from '@/components/projection/PaneSurface';

/**
 * `/me/hangar` — the whole pledge ledger.
 *
 * The `hangar` widget caps at 12 rows and reports the rest as "+N
 * more". Its own note said there was no detail page to link to, citing
 * the zero-credentials invariant. That invariant is about REFRESH
 * affordances: CLAUDE.md requires anything offering to refresh
 * hangar-style data to point at the tray rather than imply the server
 * can fetch from RSI. A read-only list of a snapshot the server
 * already holds does not fetch anything, so it does not conflict —
 * `/me/loadout` is the same shape and predates this.
 *
 * NO REFRESH CONTROL HERE, and that is the invariant showing up rather
 * than an omission. The tray owns the RSI cookie and is the only thing
 * that can re-scrape; a button here could only ever lie about it. The
 * page states when the snapshot was captured and points at the tray.
 *
 * Not range-aware, unlike contracts and travel: a hangar snapshot is a
 * point in time, not a window over events, so there is nothing for a
 * range to select.
 */
export type HangarSection = SurfaceSection;

/**
 * ONE group, deliberately.
 *
 * `PaneSurface` renders a single group at a time behind a rail, so a
 * second group would put the item list one click away — on the page
 * whose entire purpose is seeing the list. Contracts can afford to
 * split Outcomes from Runs; here the list IS the page.
 */
export const HANGAR_GROUPS: readonly SurfaceGroup[] = [
  { key: 'hangar', label: 'Hangar' },
];

export type HangarProjectionProps = Omit<
  PaneSurfaceProps,
  'crumb' | 'account' | 'groups' | 'themeAction' | 'chromeTrailing'
>;

export function HangarProjection(props: HangarProjectionProps) {
  return (
    <PaneSurface
      {...props}
      groups={HANGAR_GROUPS}
      crumb={[{ label: 'Projection', href: '/me' }, { label: 'Hangar' }]}
      account={[
        { id: 'me', label: 'Projection', href: '/me' },
        { id: 'sharing', label: 'Sharing', href: '/sharing' },
        { id: 'settings', label: 'Calibrate', href: '/settings' },
      ]}
    />
  );
}
