'use server';

/**
 * Server actions for the share-reports moderation queue
 * (audit v2 §05). One action per moderator transition; each one
 * wraps the `/v1/admin/sharing/reports/:id/resolve` POST, then
 * revalidates the queue path so the next render reflects the new
 * state.
 *
 * Error handling matches the rest of the admin surface: 401 sends
 * the user to login (with `next` set so they land back on the
 * queue), 403 boots them to the dashboard, 409 is the "someone
 * else got there first" race and falls through to a soft refresh,
 * anything else re-throws so the page boundary catches it.
 */

import { revalidatePath } from 'next/cache';
import { redirect } from 'next/navigation';
import { ApiCallError, resolveShareReport } from '@/lib/api';
import { getSession } from '@/lib/session';

const LOGIN_NEXT = '/auth/login?next=/admin/sharing/reports';
const QUEUE_PATH = '/admin/sharing/reports';

/**
 * `outcome` is supplied by the form via a hidden input, so each
 * resolution button (`Dismiss` / `Revoke share` / `Suspend owner`)
 * posts a different value to the same action.
 */
export async function resolveShareReportAction(
  formData: FormData,
): Promise<void> {
  const session = await getSession();
  if (!session) redirect(LOGIN_NEXT);

  const id = String(formData.get('id') ?? '').trim();
  const outcome = String(formData.get('outcome') ?? '').trim();
  const noteRaw = formData.get('note');
  const note =
    typeof noteRaw === 'string' && noteRaw.trim().length > 0
      ? noteRaw.trim()
      : undefined;

  if (!id || !outcome) {
    revalidatePath(QUEUE_PATH);
    return;
  }

  try {
    await resolveShareReport(session.token, id, { outcome, note });
  } catch (e) {
    if (e instanceof ApiCallError) {
      if (e.status === 401) redirect(LOGIN_NEXT);
      if (e.status === 403) redirect('/me');
      if (e.status === 409) {
        revalidatePath(QUEUE_PATH);
        return;
      }
    }
    throw e;
  }
  revalidatePath(QUEUE_PATH);
}
