'use client';

import React from 'react';
import {
  PaneSurface,
  type SurfaceSection,
  type SurfaceGroup,
  type PaneSurfaceProps,
} from '@/components/projection/PaneSurface';

/**
 * `/lfg`: the Looking for Group board, your own group, and the post form.
 * Same shell as `/friends`: short lists and forms, so the reading measure.
 * The board opens first, because finding a group is why most people come.
 */
export type LfgSection = SurfaceSection;

export const LFG_GROUPS: readonly SurfaceGroup[] = [
  { key: 'board', label: 'Board' },
  { key: 'mine', label: 'Your group' },
  { key: 'post', label: 'Post' },
  { key: 'crew', label: 'Crew' },
];

export type LfgProjectionProps = Omit<
  PaneSurfaceProps,
  'crumb' | 'account' | 'groups' | 'themeAction' | 'measure'
>;

export function LfgProjection(props: LfgProjectionProps) {
  return (
    <PaneSurface
      {...props}
      groups={LFG_GROUPS}
      measure="reading"
      crumb={[{ label: 'Projection', href: '/me' }, { label: 'Looking for Group' }]}
      account={[
        { id: 'me', label: 'Projection', href: '/me' },
        { id: 'friends', label: 'Friends', href: '/friends' },
        { id: 'lfg', label: 'Looking for Group', href: '/lfg' },
        { id: 'settings', label: 'Calibrate', href: '/settings' },
      ]}
    />
  );
}
