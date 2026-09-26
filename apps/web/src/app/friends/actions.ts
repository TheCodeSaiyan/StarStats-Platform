'use server';

import { redirect } from 'next/navigation';
import {
  ApiCallError,
  blockUser,
  markNotificationsRead,
  muteUser,
  removeFriend,
  respondToFriendRequest,
  sendFriendRequest,
  unblockUser,
  unmuteUser,
  updateSocialSettings,
} from '@/lib/api';
import { logger } from '@/lib/logger';
import { getSession } from '@/lib/session';

const LOGIN_NEXT = '/auth/login?next=/friends';

/**
 * Server error codes the page has copy for. Anything else collapses to
 * `unexpected` so a raw code never reaches the reader.
 */
const KNOWN_ERRORS = new Set([
  'invalid_handle',
  'user_not_found',
  'cannot_friend_self',
  'already_friends',
  'request_pending',
  'you_blocked_user',
  'not_accepting_requests',
  'rate_limited',
  'request_not_found',
  'request_not_pending',
  'not_friends',
  'not_blocked',
  'not_muted',
  'cannot_block_self',
  'cannot_mute_self',
]);

async function token(): Promise<string> {
  const s = await getSession();
  if (!s) redirect(LOGIN_NEXT);
  return s.token;
}

/** Map a failed call to the redirect the page understands. Never returns. */
function fail(e: unknown, call: string): never {
  if (e instanceof ApiCallError) {
    if (e.status === 401) redirect(LOGIN_NEXT);
    if (KNOWN_ERRORS.has(e.body.error)) {
      redirect(`/friends?error=${e.body.error}`);
    }
    logger.error({ err: e, call, status: e.status }, 'friends action failed');
  } else {
    logger.error({ err: e, call }, 'friends action failed');
  }
  redirect('/friends?error=unexpected');
}

function field(formData: FormData, name: string): string {
  return String(formData.get(name) ?? '').trim();
}

export async function sendFriendRequestAction(formData: FormData) {
  const t = await token();
  const handle = field(formData, 'handle').replace(/^@/, '');
  if (!handle) redirect('/friends?error=invalid_handle');
  let outcome: string;
  try {
    outcome = (await sendFriendRequest(t, handle)).outcome;
  } catch (e) {
    fail(e, 'friends.send');
  }
  // From the response, not the intent: crossing requests settle as friends.
  redirect(
    outcome === 'became_friends'
      ? '/friends?status=became_friends'
      : '/friends?status=request_sent',
  );
}

export async function respondToRequestAction(formData: FormData) {
  const t = await token();
  const id = field(formData, 'request_id');
  const action = field(formData, 'action');
  if (!id || !['accept', 'decline', 'cancel'].includes(action)) {
    redirect('/friends?error=unexpected');
  }
  try {
    await respondToFriendRequest(t, id, action as 'accept' | 'decline' | 'cancel');
  } catch (e) {
    fail(e, `friends.${action}`);
  }
  const done = { accept: 'accepted', decline: 'declined', cancel: 'cancelled' }[
    action as 'accept' | 'decline' | 'cancel'
  ];
  redirect(`/friends?status=request_${done}`);
}

export async function removeFriendAction(formData: FormData) {
  const t = await token();
  try {
    await removeFriend(t, field(formData, 'handle'));
  } catch (e) {
    fail(e, 'friends.remove');
  }
  redirect('/friends?status=friend_removed');
}

export async function blockAction(formData: FormData) {
  const t = await token();
  try {
    await blockUser(t, field(formData, 'handle'));
  } catch (e) {
    fail(e, 'friends.block');
  }
  redirect('/friends?status=blocked');
}

export async function unblockAction(formData: FormData) {
  const t = await token();
  try {
    await unblockUser(t, field(formData, 'handle'));
  } catch (e) {
    fail(e, 'friends.unblock');
  }
  redirect('/friends?status=unblocked');
}

export async function muteAction(formData: FormData) {
  const t = await token();
  try {
    await muteUser(t, field(formData, 'handle'));
  } catch (e) {
    fail(e, 'friends.mute');
  }
  redirect('/friends?status=muted');
}

export async function unmuteAction(formData: FormData) {
  const t = await token();
  try {
    await unmuteUser(t, field(formData, 'handle'));
  } catch (e) {
    fail(e, 'friends.unmute');
  }
  redirect('/friends?status=unmuted');
}

export async function policyAction(formData: FormData) {
  const t = await token();
  const wanted = field(formData, 'friend_request_policy');
  if (wanted !== 'everyone' && wanted !== 'nobody') {
    redirect('/friends?error=unexpected');
  }
  let stored: string;
  try {
    stored = (await updateSocialSettings(t, { friend_request_policy: wanted }))
      .friend_request_policy;
  } catch (e) {
    fail(e, 'friends.policy');
  }
  redirect(`/friends?status=policy_${stored}`);
}

export async function markAllReadAction() {
  const t = await token();
  try {
    await markNotificationsRead(t, { all: true });
  } catch (e) {
    fail(e, 'friends.mark_read');
  }
  redirect('/friends?status=notifications_read');
}
