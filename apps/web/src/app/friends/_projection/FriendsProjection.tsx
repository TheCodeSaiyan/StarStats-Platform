'use client';

import React from 'react';
import {
  PaneSurface,
  type SurfaceSection,
  type SurfaceGroup,
  type PaneSurfaceProps,
} from '@/components/projection/PaneSurface';

/**
 * `/friends` — friends, requests, notifications and the privacy controls
 * that go with them. Same shell as `/sharing`: forms and short lists, so the
 * reading measure rather than the wide one.
 *
 * Friendship and sharing are deliberately separate surfaces. Being someone's
 * friend grants no view of their stats; `/sharing` is still where that is
 * decided.
 */
export type FriendsSection = SurfaceSection;

export const FRIENDS_GROUPS: readonly SurfaceGroup[] = [
  { key: 'friends', label: 'Friends' },
  { key: 'requests', label: 'Requests' },
  { key: 'notifications', label: 'Notifications' },
  { key: 'privacy', label: 'Privacy' },
];

export type FriendsProjectionProps = Omit<
  PaneSurfaceProps,
  'crumb' | 'account' | 'groups' | 'themeAction' | 'measure'
>;

export function FriendsProjection(props: FriendsProjectionProps) {
  return (
    <PaneSurface
      {...props}
      groups={FRIENDS_GROUPS}
      measure="reading"
      crumb={[{ label: 'Projection', href: '/me' }, { label: 'Friends' }]}
      account={[
        { id: 'me', label: 'Projection', href: '/me' },
        { id: 'friends', label: 'Friends', href: '/friends' },
        { id: 'sharing', label: 'Sharing', href: '/sharing' },
        { id: 'settings', label: 'Calibrate', href: '/settings' },
      ]}
    />
  );
}
