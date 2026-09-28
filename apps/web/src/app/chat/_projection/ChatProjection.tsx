'use client';

import React from 'react';
import {
  PaneSurface,
  type SurfaceSection,
  type SurfaceGroup,
  type PaneSurfaceProps,
} from '@/components/projection/PaneSurface';

/**
 * `/chat`: crew rooms and friend DMs. One group; the section holds either
 * the gate (verify, declare age, restricted) or the chat itself. The wide
 * measure, because a conversation wants the room.
 */
export type ChatSection = SurfaceSection;

export const CHAT_GROUPS: readonly SurfaceGroup[] = [{ key: 'chat', label: 'Chat' }];

export type ChatProjectionProps = Omit<
  PaneSurfaceProps,
  'crumb' | 'account' | 'groups' | 'themeAction' | 'measure'
>;

export function ChatProjection(props: ChatProjectionProps) {
  return (
    <PaneSurface
      {...props}
      groups={CHAT_GROUPS}
      measure="wide"
      crumb={[{ label: 'Projection', href: '/me' }, { label: 'Chat' }]}
      account={[
        { id: 'me', label: 'Projection', href: '/me' },
        { id: 'friends', label: 'Friends', href: '/friends' },
        { id: 'lfg', label: 'Looking for Group', href: '/lfg' },
        { id: 'settings', label: 'Calibrate', href: '/settings' },
      ]}
    />
  );
}
