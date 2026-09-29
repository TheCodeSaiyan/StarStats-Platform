/**
 * Chat (social phase 6): crew rooms and friend DMs, end-to-end encrypted.
 *
 * Backend contracts:
 *  - GET  /v1/me/chat                 — which gates are open
 *  - POST /v1/me/chat/age-declaration — the age declaration
 *  - GET  /v1/me/chat/rooms           — rooms and what each is for
 *  - GET  /v1/me/friends              — who a new message can go to
 *  - POST /v1/me/matrix/login-token   — via chatLoginAction, for the browser
 *
 * Hidden entirely unless STARSTATS_CHAT_ENABLED allows this account
 * (`lib/chat/flag`). The messages themselves never pass through StarStats:
 * the browser talks to the homeserver (components/chat/ChatApp).
 */
import { notFound, redirect } from 'next/navigation';
import React from 'react';
import { BeamAlert, BeamButton } from 'holo';
import type { Calibration } from 'holo';
import {
  ApiCallError,
  getChatStatus,
  getFriends,
  getMyChatRooms,
  type ChatStatus,
  type Friend,
  type MyChatRoom,
} from '@/lib/api';
import { chatEnabledFor } from '@/lib/chat/flag';
import { logger } from '@/lib/logger';
import { navSections } from '@/lib/nav';
import { getSession } from '@/lib/session';
import { getTheme } from '@/lib/theme';
import { setCalibrationAction } from '@/app/me/_projection/actions';
import { ChatApp } from '@/components/chat/ChatApp';
import { ChatProjection, type ChatSection } from './_projection/ChatProjection';
import { declareAgeAction } from './actions';
import { StartChat } from './StartChat';

export const metadata = { title: 'Chat' };

const ERROR_MESSAGES: Record<string, string> = {
  confirm_age: 'Tick the box to confirm your age.',
  wrong_minimum_age: 'The minimum age changed. Reload and try again.',
  rsi_handle_not_verified: 'Verify your RSI handle first (Calibrate → RSI handle).',
  age_not_declared: 'Confirm your age to use chat.',
  chat_restricted: 'Your account is restricted from chat.',
  chat_unavailable: 'Chat is not available right now.',
  not_friends: 'You can message friends only.',
  cannot_message: 'That player cannot be messaged.',
  cannot_message_self: 'You cannot message yourself.',
  rate_limited: 'That is a lot at once. Try again in a minute.',
  unexpected: 'Something went wrong. Try again.',
};

type SearchParams = { room?: string; error?: string };

export default async function ChatPage(props: { searchParams: Promise<SearchParams> }) {
  const session = await getSession();
  if (!session) redirect('/auth/login?next=/chat');
  if (!chatEnabledFor(session)) notFound();
  const params = await props.searchParams;

  let calibration: Calibration = 'terra';
  try {
    calibration = (await getTheme(session.token)) as Calibration;
  } catch (e) {
    logger.warn({ err: e, call: 'chat.theme' }, 'load theme failed');
  }

  const [statusRes, roomsRes, friendsRes] = await Promise.allSettled([
    getChatStatus(session.token),
    getMyChatRooms(session.token),
    getFriends(session.token),
  ]);
  for (const [r, call] of [
    [statusRes, 'chat.status'],
    [roomsRes, 'chat.rooms'],
    [friendsRes, 'chat.friends'],
  ] as const) {
    if (r.status === 'rejected') {
      const status = r.reason instanceof ApiCallError ? r.reason.status : undefined;
      if (status === 401) redirect('/auth/login?next=/chat');
      logger.error({ err: r.reason, call, status }, 'chat page call failed');
    }
  }
  const status: ChatStatus | null = statusRes.status === 'fulfilled' ? statusRes.value : null;
  const rooms: MyChatRoom[] = roomsRes.status === 'fulfilled' ? roomsRes.value.rooms : [];
  const friends: Friend[] | null =
    friendsRes.status === 'fulfilled' ? friendsRes.value.friends : null;

  let node: React.ReactNode;
  if (!status) {
    node = <BeamAlert tone="bad">Couldn&apos;t load chat. Refresh to retry.</BeamAlert>;
  } else if (!status.available) {
    node = <p className="hp-prose">Chat is not switched on yet.</p>;
  } else if (status.restricted) {
    node = <p className="hp-prose">Your account is restricted from chat.</p>;
  } else if (!status.rsi_verified) {
    node = (
      <p className="hp-prose">
        Chat needs a verified RSI handle, so every player you talk to is who they say they are.
        Verify yours on Calibrate → RSI handle.
      </p>
    );
  } else if (!status.age_declared) {
    node = (
      <form action={declareAgeAction} className="hp-formcol" data-testid="age-declaration">
        <p className="hp-prose">
          Chat is for players aged {status.minimum_age} and over, in line with the game&apos;s own
          terms. We record that you confirmed it, and when.
        </p>
        <input type="hidden" name="minimum_age" value={status.minimum_age} />
        <label className="hp-check">
          <input type="checkbox" name="confirm" value="yes" required /> I am {status.minimum_age}{' '}
          or over
        </label>
        <BeamButton type="submit" variant="primary" style={{ alignSelf: 'flex-start' }}>
          Continue
        </BeamButton>
      </form>
    );
  } else {
    node = (
      <>
        {/* If the friend list failed to load, say nothing rather than
            claim the player has no friends. */}
        {friends ? <StartChat friends={friends} /> : null}
        <ChatApp handle={session.claimedHandle} rooms={rooms} initialRoom={params.room} />
      </>
    );
  }

  const sections: ChatSection[] = [{ id: 'chat', title: 'Chat', group: 'chat', node }];
  const notice = params.error
    ? { tone: 'bad' as const, message: ERROR_MESSAGES[params.error] ?? ERROR_MESSAGES.unexpected }
    : null;

  return (
    <ChatProjection
      handle={session.claimedHandle}
      calibration={calibration}
      nav={navSections({ signedIn: true, staffRoles: session.staffRoles }, 'chat')}
      sections={sections}
      notice={notice}
      onCalibrate={async (id: string) => {
        'use server';
        await setCalibrationAction(id);
      }}
    />
  );
}
