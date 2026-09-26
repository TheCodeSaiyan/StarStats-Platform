/**
 * Friends — friends, requests, notifications, blocks and mutes.
 *
 * Backend contracts:
 *  - GET  /v1/me/friends                        — friends + pending both ways + policy
 *  - POST /v1/me/friends/requests {handle}      — send (or settle a crossing request)
 *  - POST /v1/me/friends/requests/:id/:action   — accept | decline | cancel
 *  - DEL  /v1/me/friends/:handle                — unfriend
 *  - GET/PUT/DEL /v1/me/blocks[/:handle]        — block list
 *  - GET/PUT/DEL /v1/me/mutes[/:handle]         — mute list
 *  - PUT  /v1/me/social/settings                — who may send requests
 *  - GET  /v1/me/notifications                  — inbox + unread count
 *  - POST /v1/me/notifications/read             — mark read
 *
 * Every section is fed by its own call through `allSettled`, so one failing
 * endpoint blanks one section, not the page.
 */

import Link from 'next/link';
import type { Route } from 'next';
import { redirect } from 'next/navigation';
import React from 'react';
import { BeamAlert, BeamButton, BeamChip, BeamInput, BeamSelect, Plane } from 'holo';
import type { Calibration } from 'holo';
import {
  ApiCallError,
  getFriends,
  getNotifications,
  listBlocks,
  listMutes,
  type AppNotification,
  type BlocksResponse,
  type FriendsResponse,
  type MutesResponse,
  type NotificationsResponse,
} from '@/lib/api';
import { logger } from '@/lib/logger';
import { navSections } from '@/lib/nav';
import { getSession } from '@/lib/session';
import { getTheme } from '@/lib/theme';
import { setCalibrationAction } from '@/app/me/_projection/actions';
import { formatRelativePast } from '@/app/sharing/_projection/format';
import { ConfirmSubmitButton } from '@/components/forms/ConfirmSubmitButton';
import { CopyHandleButton } from '@/components/social/CopyHandleButton';
import { FriendsProjection, type FriendsSection } from './_projection/FriendsProjection';
import {
  blockAction,
  markAllReadAction,
  muteAction,
  policyAction,
  removeFriendAction,
  respondToRequestAction,
  sendFriendRequestAction,
  unblockAction,
  unmuteAction,
} from './actions';

export const metadata = { title: 'Friends' };

const STATUS_MESSAGES: Record<string, string> = {
  request_sent: 'Friend request sent.',
  became_friends: 'They had already asked — you are now friends.',
  request_accepted: 'Request accepted — you are now friends.',
  request_declined: 'Request declined. They are not told.',
  request_cancelled: 'Request withdrawn.',
  friend_removed: 'Removed from your friends.',
  blocked: 'Blocked. They are not told.',
  unblocked: 'Unblocked.',
  muted: 'Muted — their activity no longer notifies you.',
  unmuted: 'Unmuted.',
  policy_everyone: 'Anyone can now send you friend requests.',
  policy_nobody: 'Nobody can send you friend requests now.',
  notifications_read: 'All notifications marked read.',
};

const ERROR_MESSAGES: Record<string, string> = {
  invalid_handle: 'Handle looks invalid — letters, digits, underscores and dashes only.',
  user_not_found: 'No StarStats account exists for that handle.',
  cannot_friend_self: "You can't send yourself a friend request.",
  already_friends: 'You are already friends.',
  request_pending: 'You already have a request waiting with them.',
  you_blocked_user: 'You have blocked that user. Unblock them first.',
  not_accepting_requests: 'That user is not accepting friend requests.',
  rate_limited: "You've sent a lot of requests today. Try again tomorrow.",
  request_not_found: 'That request no longer exists.',
  request_not_pending: 'That request has already been answered.',
  not_friends: 'You are not friends with that user.',
  not_blocked: 'That user is not blocked.',
  not_muted: 'That user is not muted.',
  cannot_block_self: "You can't block yourself.",
  cannot_mute_self: "You can't mute yourself.",
  unexpected: 'Something went wrong. Try again.',
};

type SearchParams = { status?: string; error?: string };

export default async function FriendsPage(props: {
  searchParams: Promise<SearchParams>;
}) {
  const session = await getSession();
  if (!session) redirect('/auth/login?next=/friends');
  const params = await props.searchParams;

  let calibration: Calibration = 'terra';
  try {
    calibration = (await getTheme(session.token)) as Calibration;
  } catch (e) {
    logger.warn({ err: e, call: 'friends.theme' }, 'load theme failed');
  }

  const calls = [
    ['friends', getFriends(session.token)],
    ['blocks', listBlocks(session.token)],
    ['mutes', listMutes(session.token)],
    ['notifications', getNotifications(session.token, { limit: 50 })],
  ] as const;
  const settled = await Promise.allSettled(calls.map(([, p]) => p));
  settled.forEach((r, i) => {
    if (r.status === 'rejected') {
      const status = r.reason instanceof ApiCallError ? r.reason.status : undefined;
      if (status === 401) redirect('/auth/login?next=/friends');
      logger.error(
        { err: r.reason, call: `friends.${calls[i][0]}`, status },
        'friends page call failed',
      );
    }
  });
  const value = <T,>(i: number): T | null =>
    settled[i].status === 'fulfilled' ? (settled[i].value as T) : null;
  const friends = value<FriendsResponse>(0);
  const blocks = value<BlocksResponse>(1);
  const mutes = value<MutesResponse>(2);
  const notes = value<NotificationsResponse>(3);

  const unavailable = (what: string) => (
    <BeamAlert tone="bad">Couldn&apos;t load {what}. Refresh to retry.</BeamAlert>
  );

  const sections: FriendsSection[] = [
    {
      id: 'add',
      title: 'Add a friend',
      group: 'friends',
      node: (
        <form action={sendFriendRequestAction} className="hp-formcol">
          <BeamInput
            id="friend-handle"
            name="handle"
            label="Their StarStats handle"
            hint="They get a request to accept. Being friends shares no stats — that stays on Sharing."
            required
            maxLength={64}
            autoComplete="off"
          />
          <BeamButton type="submit" variant="primary" style={{ alignSelf: 'flex-start' }}>
            Send request
          </BeamButton>
        </form>
      ),
    },
    {
      id: 'list',
      title: 'Friends',
      ctx: friends ? String(friends.friends.length) : undefined,
      group: 'friends',
      node: !friends ? (
        unavailable('your friends')
      ) : friends.friends.length === 0 ? (
        <p className="hp-prose">No friends yet. Send a request above.</p>
      ) : (
        <Plane tilt="flat" style={{ marginTop: 18 }}>
          {friends.friends.map((f) => (
            <div className="hp-grant" key={f.handle}>
              <div className="hp-grant__who">
                <Link href={`/u/${encodeURIComponent(f.handle)}` as Route}>@{f.handle}</Link>
                <span className="hp-grant__note">
                  Friends since {formatRelativePast(f.since) ?? 'recently'}
                  {f.rsi_verified ? '' : ' · RSI handle not verified'}
                </span>
              </div>
              <div className="hp-grant__act-btns">
                <CopyHandleButton handle={f.handle} verified={f.rsi_verified} />
                <CopyHandleButton handle={f.handle} verified={f.rsi_verified} addInGame />
              </div>
              <details className="hp-report">
                <summary>More</summary>
                <div className="ss-social-actions">
                  <form action={removeFriendAction}>
                    <input type="hidden" name="handle" value={f.handle} />
                    <ConfirmSubmitButton
                      className="hp-btn hp-btn--ghost"
                      confirm={`Remove @${f.handle} from your friends?`}
                    >
                      Unfriend
                    </ConfirmSubmitButton>
                  </form>
                  <form action={muteAction}>
                    <input type="hidden" name="handle" value={f.handle} />
                    <ConfirmSubmitButton className="hp-btn hp-btn--ghost">Mute</ConfirmSubmitButton>
                  </form>
                  <form action={blockAction}>
                    <input type="hidden" name="handle" value={f.handle} />
                    <ConfirmSubmitButton
                      className="hp-btn hp-btn--danger"
                      confirm={`Block @${f.handle}? This also unfriends them and revokes any stats share you gave them.`}
                    >
                      Block
                    </ConfirmSubmitButton>
                  </form>
                </div>
              </details>
            </div>
          ))}
        </Plane>
      ),
    },
    {
      id: 'incoming',
      title: 'Incoming requests',
      ctx: friends ? String(friends.incoming.length) : undefined,
      group: 'requests',
      node: !friends ? (
        unavailable('your requests')
      ) : friends.incoming.length === 0 ? (
        <p className="hp-prose">No requests waiting.</p>
      ) : (
        <Plane tilt="flat" style={{ marginTop: 18 }}>
          {friends.incoming.map((r) => (
            <div className="hp-grant" key={r.id}>
              <div className="hp-grant__who">
                <Link href={`/u/${encodeURIComponent(r.requester_handle)}` as Route}>
                  @{r.requester_handle}
                </Link>
                <span className="hp-grant__note">
                  Asked {formatRelativePast(r.created_at) ?? 'recently'}
                </span>
              </div>
              <div className="ss-social-actions">
                <form action={respondToRequestAction}>
                  <input type="hidden" name="request_id" value={r.id} />
                  <input type="hidden" name="action" value="accept" />
                  <ConfirmSubmitButton className="hp-btn hp-btn--primary">Accept</ConfirmSubmitButton>
                </form>
                <form action={respondToRequestAction}>
                  <input type="hidden" name="request_id" value={r.id} />
                  <input type="hidden" name="action" value="decline" />
                  <ConfirmSubmitButton className="hp-btn hp-btn--ghost">Decline</ConfirmSubmitButton>
                </form>
                <form action={blockAction}>
                  <input type="hidden" name="handle" value={r.requester_handle} />
                  <ConfirmSubmitButton
                    className="hp-btn hp-btn--danger"
                    confirm={`Block @${r.requester_handle}? They are not told.`}
                  >
                    Block
                  </ConfirmSubmitButton>
                </form>
              </div>
            </div>
          ))}
        </Plane>
      ),
    },
    {
      id: 'outgoing',
      title: 'Sent requests',
      ctx: friends ? String(friends.outgoing.length) : undefined,
      group: 'requests',
      node: !friends ? (
        unavailable('your sent requests')
      ) : friends.outgoing.length === 0 ? (
        <p className="hp-prose">Nothing waiting on anyone else.</p>
      ) : (
        <Plane tilt="flat" style={{ marginTop: 18 }}>
          {friends.outgoing.map((r) => (
            <div className="hp-grant" key={r.id}>
              <div className="hp-grant__who">
                <span>@{r.recipient_handle}</span>
                <span className="hp-grant__note">
                  Sent {formatRelativePast(r.created_at) ?? 'recently'}
                </span>
              </div>
              <form action={respondToRequestAction}>
                <input type="hidden" name="request_id" value={r.id} />
                <input type="hidden" name="action" value="cancel" />
                <ConfirmSubmitButton className="hp-btn hp-btn--ghost">Withdraw</ConfirmSubmitButton>
              </form>
            </div>
          ))}
        </Plane>
      ),
    },
    {
      id: 'inbox',
      title: 'Notifications',
      ctx: notes ? `${notes.unread_count} unread` : undefined,
      group: 'notifications',
      node: !notes ? (
        unavailable('your notifications')
      ) : notes.items.length === 0 ? (
        <p className="hp-prose">Nothing yet. Friend requests and replies show up here.</p>
      ) : (
        <>
          {notes.unread_count > 0 ? (
            <form action={markAllReadAction} style={{ marginTop: 14 }}>
              <ConfirmSubmitButton className="hp-btn hp-btn--ghost">Mark all read</ConfirmSubmitButton>
            </form>
          ) : null}
          <Plane tilt="flat" style={{ marginTop: 18 }}>
            {notes.items.map((n) => (
              <NotificationRow key={n.id} n={n} />
            ))}
          </Plane>
        </>
      ),
    },
    {
      id: 'policy',
      title: 'Who can send you requests',
      group: 'privacy',
      node: !friends ? (
        unavailable('your settings')
      ) : (
        <form action={policyAction} className="hp-formcol">
          <BeamSelect
            id="friend-request-policy"
            name="friend_request_policy"
            label="Friend requests"
            defaultValue={friends.friend_request_policy}
          >
            <option value="everyone">Anyone with a StarStats account</option>
            <option value="nobody">Nobody</option>
          </BeamSelect>
          <BeamButton type="submit" style={{ alignSelf: 'flex-start' }}>
            Save
          </BeamButton>
        </form>
      ),
    },
    {
      id: 'blocked',
      title: 'Blocked',
      ctx: blocks ? String(blocks.blocks.length) : undefined,
      group: 'privacy',
      node: !blocks ? (
        unavailable('your block list')
      ) : (
        <>
          <p className="hp-prose">
            A blocked user can&apos;t tell they are blocked. Their requests are
            hidden from you and they stop notifying you.
          </p>
          <HandleList
            entries={blocks.blocks}
            action={unblockAction}
            button="Unblock"
            empty="You haven't blocked anyone."
          />
        </>
      ),
    },
    {
      id: 'muted',
      title: 'Muted',
      ctx: mutes ? String(mutes.mutes.length) : undefined,
      group: 'privacy',
      node: !mutes ? (
        unavailable('your mute list')
      ) : (
        <>
          <p className="hp-prose">
            Muting only silences notifications. Friendships and requests are
            unchanged.
          </p>
          <HandleList
            entries={mutes.mutes}
            action={unmuteAction}
            button="Unmute"
            empty="You haven't muted anyone."
          />
        </>
      ),
    },
  ];

  const notice = params.status && STATUS_MESSAGES[params.status]
    ? { tone: 'good' as const, message: STATUS_MESSAGES[params.status] }
    : params.error
      ? {
          tone: 'bad' as const,
          message: ERROR_MESSAGES[params.error] ?? ERROR_MESSAGES.unexpected,
        }
      : null;

  return (
    <FriendsProjection
      handle={session.claimedHandle}
      calibration={calibration}
      nav={navSections({ signedIn: true, staffRoles: session.staffRoles }, 'friends')}
      sections={sections}
      notice={notice}
      onCalibrate={async (id: string) => {
        'use server';
        await setCalibrationAction(id);
      }}
    />
  );
}

function NotificationRow({ n }: { n: AppNotification }) {
  const who = n.actor_handle ?? 'Someone';
  const payload = (n.payload ?? {}) as { request_id?: string; rsi_verified?: boolean };
  const text =
    n.kind === 'friend_request'
      ? `@${who} sent you a friend request`
      : `@${who} accepted your friend request`;
  return (
    <div className="hp-grant" data-unread={n.read_at ? undefined : 'true'}>
      <div className="hp-grant__who">
        <span>{text}</span>
        <span className="hp-grant__note">{formatRelativePast(n.created_at) ?? ''}</span>
      </div>
      {n.read_at ? null : <BeamChip tone="warn">new</BeamChip>}
      <div className="hp-grant__act-btns">
        {n.kind === 'friend_request' && payload.request_id ? (
          <form action={respondToRequestAction}>
            <input type="hidden" name="request_id" value={payload.request_id} />
            <input type="hidden" name="action" value="accept" />
            <ConfirmSubmitButton className="hp-btn hp-btn--primary">Accept</ConfirmSubmitButton>
          </form>
        ) : null}
        {n.actor_handle ? (
          <CopyHandleButton
            handle={n.actor_handle}
            verified={payload.rsi_verified === true}
            addInGame
          />
        ) : null}
      </div>
    </div>
  );
}

function HandleList({
  entries,
  action,
  button,
  empty,
}: {
  entries: readonly { handle: string; since: string }[];
  action: (formData: FormData) => void | Promise<void>;
  button: string;
  empty: string;
}) {
  if (entries.length === 0) return <p className="hp-prose">{empty}</p>;
  return (
    <Plane tilt="flat" style={{ marginTop: 18 }}>
      {entries.map((e) => (
        <div className="hp-grant" key={e.handle}>
          <div className="hp-grant__who">
            <span>@{e.handle}</span>
            <span className="hp-grant__note">
              Since {formatRelativePast(e.since) ?? 'recently'}
            </span>
          </div>
          <form action={action}>
            <input type="hidden" name="handle" value={e.handle} />
            <ConfirmSubmitButton className="hp-btn hp-btn--ghost">{button}</ConfirmSubmitButton>
          </form>
        </div>
      ))}
    </Plane>
  );
}
