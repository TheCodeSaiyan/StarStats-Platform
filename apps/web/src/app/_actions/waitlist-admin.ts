'use server';

import { revalidatePath } from 'next/cache';
import { getSession } from '@/lib/session';
import {
  admitWaitlist,
  deleteWaitlist,
  resendWaitlist,
  setWaitlistConfig,
  type WaitlistConfigApi,
} from '@/lib/api';
import { logger } from '@/lib/logger';

export type AdmitResult =
  | { ok: true; admitted: number }
  | { ok: false; error: string };

/**
 * Admit a batch of queued signups. Each admitted row mints an invite and
 * the server mails it.
 *
 * `admitted` can be lower than `ids.length` — rows already admitted are
 * skipped so a double-click never re-mints over a live invite. The caller
 * should show the returned count rather than assume every id took.
 */
export async function admitWaitlistAction(ids: string[]): Promise<AdmitResult> {
  const session = await getSession();
  if (!session) return { ok: false, error: 'not_authenticated' };
  if (ids.length === 0) return { ok: true, admitted: 0 };
  try {
    const { admitted } = await admitWaitlist(session.token, ids);
    revalidatePath('/admin/waitlist');
    return { ok: true, admitted };
  } catch (err) {
    logger.warn({ err, call: 'action.admitWaitlist' }, 'admit failed');
    return { ok: false, error: 'admit_failed' };
  }
}

export type ResendResult =
  | { ok: true; resent: number }
  | { ok: false; error: string };

/**
 * Re-send invites to already-admitted rows whose mail failed the first
 * time. Uses the existing token (no re-mint), so a link already delivered
 * stays valid. `resent` counts successful sends — if it comes back lower
 * than the number selected, the transport is still broken and the console
 * must say so rather than claim success.
 */
export async function resendWaitlistAction(
  ids: string[],
): Promise<ResendResult> {
  const session = await getSession();
  if (!session) return { ok: false, error: 'not_authenticated' };
  if (ids.length === 0) return { ok: true, resent: 0 };
  try {
    const { resent } = await resendWaitlist(session.token, ids);
    revalidatePath('/admin/waitlist');
    return { ok: true, resent };
  } catch (err) {
    logger.warn({ err, call: 'action.resendWaitlist' }, 'resend failed');
    return { ok: false, error: 'resend_failed' };
  }
}

export type DeleteResult =
  | { ok: true; deleted: string[]; blocked: string[] }
  | { ok: false; error: string };

/**
 * Permanently delete waitlist signups. Rows whose invite was already
 * redeemed are refused by the server and come back in `blocked` — as ids,
 * not a count, so the console can say WHICH rows were refused rather than
 * leave an admin guessing (see `DeleteOutcome`'s doc comment in
 * waitlist.rs). Collapsing these to `.length` here would throw the ids
 * away before the console ever sees them.
 */
export async function deleteWaitlistAction(
  ids: string[],
): Promise<DeleteResult> {
  const session = await getSession();
  if (!session) return { ok: false, error: 'not_authenticated' };
  if (ids.length === 0) return { ok: true, deleted: [], blocked: [] };
  try {
    const { deleted, blocked } = await deleteWaitlist(session.token, ids);
    revalidatePath('/admin/waitlist');
    return { ok: true, deleted, blocked };
  } catch (err) {
    logger.warn({ err, call: 'action.deleteWaitlist' }, 'delete failed');
    return { ok: false, error: 'delete_failed' };
  }
}

export type SaveConfigResult =
  | { ok: true; config: WaitlistConfigApi }
  | { ok: false; error: string };

export async function saveWaitlistConfigAction(
  cfg: WaitlistConfigApi,
): Promise<SaveConfigResult> {
  const session = await getSession();
  if (!session) return { ok: false, error: 'not_authenticated' };
  try {
    // The server clamps and echoes what it STORED — return that, not
    // what was sent, or the console shows a cap the DB is not enforcing.
    const config = await setWaitlistConfig(session.token, cfg);
    revalidatePath('/admin/waitlist');
    return { ok: true, config };
  } catch (err) {
    logger.warn({ err, call: 'action.saveWaitlistConfig' }, 'save config failed');
    return { ok: false, error: 'save_failed' };
  }
}
