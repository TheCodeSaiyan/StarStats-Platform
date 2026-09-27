'use server';

import { redirect } from 'next/navigation';
import {
  ApiCallError,
  closeLfgPost,
  commendCrewmate,
  createLfgPost,
  joinLfgPost,
  leaveLfgPost,
  reportLfgPost,
  respondToLfgMember,
  withdrawCommend,
  type CommendKind,
  type CreateLfgPost,
} from '@/lib/api';
import { COMMEND_KINDS } from '@/lib/commends';
import { logger } from '@/lib/logger';
import { getSession } from '@/lib/session';

const LOGIN_NEXT = '/auth/login?next=/lfg';

/** Server error codes the page has copy for; anything else is `unexpected`. */
const KNOWN_ERRORS = new Set([
  'rsi_handle_not_verified',
  'invalid_system',
  'invalid_crew_slots',
  'invalid_expiry',
  'location_too_long',
  'ship_too_long',
  'note_too_long',
  'details_too_long',
  'location_invalid',
  'ship_invalid',
  'note_invalid',
  'already_posting',
  'rate_limited',
  'not_found',
  'own_post',
  'already_asked',
  'group_full',
  'removed_from_group',
  'post_ended',
  'not_requested',
  'account_restricted',
  'post_not_ended',
  'window_closed',
  'cannot_commend_self',
]);

async function token(): Promise<string> {
  const s = await getSession();
  if (!s) redirect(LOGIN_NEXT);
  return s.token;
}

function fail(e: unknown, call: string): never {
  if (e instanceof ApiCallError) {
    if (e.status === 401) redirect(LOGIN_NEXT);
    if (KNOWN_ERRORS.has(e.body.error)) redirect(`/lfg?error=${e.body.error}`);
    // The restriction guard answers 403 with its own body.
    if (e.status === 403) redirect('/lfg?error=account_restricted');
    logger.error({ err: e, call, status: e.status }, 'lfg action failed');
  } else {
    logger.error({ err: e, call }, 'lfg action failed');
  }
  redirect('/lfg?error=unexpected');
}

function field(formData: FormData, name: string): string {
  return String(formData.get(name) ?? '').trim();
}

const optional = (v: string): string | undefined => (v === '' ? undefined : v);

export async function createPostAction(formData: FormData) {
  const t = await token();
  const body: CreateLfgPost = {
    activity: field(formData, 'activity') as CreateLfgPost['activity'],
    system: optional(field(formData, 'system')),
    location: optional(field(formData, 'location')),
    ship: optional(field(formData, 'ship')),
    crew_slots: Number(field(formData, 'crew_slots')),
    voice: field(formData, 'voice') as CreateLfgPost['voice'],
    region: field(formData, 'region') as CreateLfgPost['region'],
    note: optional(field(formData, 'note')),
    expires_in_minutes: Number(field(formData, 'expires_in_minutes')) || undefined,
  };
  try {
    await createLfgPost(t, body);
  } catch (e) {
    fail(e, 'lfg.create');
  }
  redirect('/lfg?status=posted');
}

export async function closePostAction(formData: FormData) {
  const t = await token();
  try {
    await closeLfgPost(t, field(formData, 'id'));
  } catch (e) {
    fail(e, 'lfg.close');
  }
  redirect('/lfg?status=closed');
}

export async function joinAction(formData: FormData) {
  const t = await token();
  try {
    await joinLfgPost(t, field(formData, 'id'));
  } catch (e) {
    fail(e, 'lfg.join');
  }
  redirect('/lfg?status=asked');
}

export async function leaveAction(formData: FormData) {
  const t = await token();
  try {
    await leaveLfgPost(t, field(formData, 'id'));
  } catch (e) {
    fail(e, 'lfg.leave');
  }
  redirect('/lfg?status=left');
}

export async function respondAction(formData: FormData) {
  const t = await token();
  const action = field(formData, 'action');
  if (action !== 'accept' && action !== 'decline' && action !== 'remove') {
    redirect('/lfg?error=unexpected');
  }
  let stored: string;
  try {
    stored = (
      await respondToLfgMember(t, field(formData, 'id'), field(formData, 'handle'), action)
    ).status;
  } catch (e) {
    fail(e, 'lfg.respond');
  }
  // From the response: the chip says what the server recorded.
  redirect(`/lfg?status=member_${stored}`);
}

export async function reportAction(formData: FormData) {
  const t = await token();
  try {
    await reportLfgPost(t, field(formData, 'id'), {
      reason: field(formData, 'reason'),
      details: optional(field(formData, 'details')),
    });
  } catch (e) {
    fail(e, 'lfg.report');
  }
  redirect('/lfg?status=reported');
}

/** Give, change or withdraw a commend for a crewmate. */
export async function commendAction(formData: FormData) {
  const t = await token();
  const postId = field(formData, 'post_id');
  const handle = field(formData, 'handle');
  const kind = field(formData, 'kind');
  const withdraw = field(formData, 'intent') === 'withdraw';
  if (!withdraw && !(COMMEND_KINDS as readonly string[]).includes(kind)) {
    redirect('/lfg?error=unexpected');
  }
  try {
    if (withdraw) {
      await withdrawCommend(t, postId, handle);
    } else {
      await commendCrewmate(t, postId, handle, kind as CommendKind);
    }
  } catch (e) {
    fail(e, 'lfg.commend');
  }
  redirect(`/lfg?status=${withdraw ? 'commend_withdrawn' : 'commended'}`);
}
