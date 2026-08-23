'use client';

import React from 'react';
import {
  PaneSurface,
  type SurfaceSection,
  type SurfaceGroup,
  type PaneSurfaceProps,
} from '@/components/projection/PaneSurface';

/**
 * `/me/loadout` — the last in-game kit you spawned with.
 *
 * "Loadout" here is the GAME's word: weapons, armour, undersuit. It is not the
 * projection layout (which readouts you show) — the system keeps those two
 * apart deliberately, because Star Citizen already owns this meaning.
 *
 * ONE rail group, so the rail hides itself: the paperdoll and the carried gear
 * are one view of one kit, and putting them behind a two-item rail would make
 * a reader click to see half of what the flat page showed at once.
 *
 * NOT range-aware. A loadout is a SNAPSHOT — the fullest recent restore burst
 * — not a series, so there is no range control to offer and offering one would
 * imply a scoping that does not exist.
 */
export type LoadoutSection = SurfaceSection;

export const LOADOUT_GROUPS: readonly SurfaceGroup[] = [
  { key: 'kit', label: 'Kit' },
];

export type LoadoutProjectionProps = Omit<
  PaneSurfaceProps,
  'crumb' | 'account' | 'groups' | 'themeAction' | 'chromeTrailing'
>;

export function LoadoutProjection(props: LoadoutProjectionProps) {
  return (
    <PaneSurface
      {...props}
      groups={LOADOUT_GROUPS}
      crumb={[{ label: 'Projection', href: '/me' }, { label: 'Loadout' }]}
      account={[
        { id: 'me', label: 'Projection', href: '/me' },
        { id: 'sharing', label: 'Sharing', href: '/sharing' },
        { id: 'settings', label: 'Calibrate', href: '/settings' },
      ]}
    />
  );
}
