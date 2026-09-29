'use server';

import { redirect } from 'next/navigation';
import {
  ApiCallError,
  declareChatAge,
  getMatrixLoginToken,
  openDm,
  reportChat,
  type MatrixLoginToken,
  type ReportChat,
} from '@/lib/api';
import { chatEnabledFor } from '@/lib/chat/flag';
import { logger } from '@/lib/logger';
import { getSession } from '@/lib/session';

const LOGIN_NEXT = '/auth/login?next=/chat';

/** Error codes the chat page has copy for; anything else is `unexpected`. */
const KNOWN_ERRORS = new Set([
  'rsi_handle_not_verified',
  'age_not_declared',
  'chat_restricted',
  'chat_unavailable',
  'not_friends',
  'cannot_message',
  'cannot_message_self',
  'wrong_minimum_age',
  'rate_limited',
]);

async function chatSession() {
  const s = await getSession();
  if (!s) redirect(LOGIN_NEXT);
  if (!chatEnabledFor(s)) redirect('/');
  return s;
}

function fail(e: unknown, call: string, back: '/chat' | '/friends'): never {
  if (e instanceof ApiCallError) {
    if (e.status === 401) redirect(LOGIN_NEXT);
    if (KNOWN_ERRORS.has(e.body.error)) {
      if (back === '/chat') redirect(`/chat?error=${e.body.error}`);
      redirect(`/friends?error=${e.body.error}`);
    }
    logger.error({ err: e, call, status: e.status }, 'chat action failed');
  } else {
    logger.error({ err: e, call }, 'chat action failed');
  }
  if (back === '/chat') redirect('/chat?error=unexpected');
  redirect('/friends?error=unexpected');
}

/** Record the age declaration. The minimum comes from the form, which the
 * page fills from the API's status, and the API accepts only the current
 * one. */
export async function declareAgeAction(formData: FormData) {
  const s = await chatSession();
  if (formData.get('confirm') !== 'yes') redirect('/chat?error=confirm_age');
  const minimum = Number(formData.get('minimum_age'));
  try {
    await declareChatAge(s.token, minimum);
  } catch (e) {
    fail(e, 'chat.declare_age', '/chat');
  }
  redirect('/chat');
}

/** A login token for the browser's Matrix client. Returned, not stored:
 * it lives a minute and is exchanged at once. */
export async function chatLoginAction(): Promise<
  { ok: true; login: MatrixLoginToken } | { ok: false; error: string }
> {
  const s = await getSession();
  if (!s || !chatEnabledFor(s)) return { ok: false, error: 'chat_unavailable' };
  try {
    return { ok: true, login: await getMatrixLoginToken(s.token) };
  } catch (e) {
    if (e instanceof ApiCallError && KNOWN_ERRORS.has(e.body.error)) {
      return { ok: false, error: e.body.error };
    }
    logger.error({ err: e, call: 'chat.login_token' }, 'chat login token failed');
    return { ok: false, error: 'unexpected' };
  }
}

/** Open (or find) the DM with a friend, then show it. */
export async function openDmAction(formData: FormData) {
  const s = await chatSession();
  const handle = String(formData.get('handle') ?? '').trim();
  // Errors go back to where the player asked from.
  const back = formData.get('from') === 'chat' ? '/chat' : '/friends';
  let room: string;
  try {
    room = (await openDm(s.token, handle)).room_id;
  } catch (e) {
    fail(e, 'chat.open_dm', back);
  }
  redirect(`/chat?room=${encodeURIComponent(room)}`);
}

/** Report a player, with the messages the reporter chose to reveal.
 * Called from the chat client, so it answers rather than redirecting. */
export async function reportChatAction(
  report: ReportChat,
): Promise<{ ok: true } | { ok: false; error: string }> {
  const s = await getSession();
  if (!s || !chatEnabledFor(s)) return { ok: false, error: 'chat_unavailable' };
  try {
    await reportChat(s.token, report);
    return { ok: true };
  } catch (e) {
    if (e instanceof ApiCallError && typeof e.body.error === 'string' && e.status < 500) {
      return { ok: false, error: e.body.error };
    }
    logger.error({ err: e, call: 'chat.report' }, 'chat report failed');
    return { ok: false, error: 'unexpected' };
  }
}
