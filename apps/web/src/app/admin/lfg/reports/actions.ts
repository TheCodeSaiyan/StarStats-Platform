'use server';

/**
 * Server action for the Looking for Group report queue. Same shape as the
 * share-report queue's: 401 to login, 403 to /me, 409 (someone else got
 * there first) is a soft refresh, anything else is thrown to the page
 * boundary.
 */

import { revalidatePath } from 'next/cache';
import { redirect } from 'next/navigation';
import { ApiCallError, resolveLfgReport } from '@/lib/api';
import { getSession } from '@/lib/session';

const LOGIN_NEXT = '/auth/login?next=/admin/lfg/reports';
const QUEUE_PATH = '/admin/lfg/reports';

export async function resolveLfgReportAction(formData: FormData): Promise<void> {
  const session = await getSession();
  if (!session) redirect(LOGIN_NEXT);

  const id = String(formData.get('id') ?? '').trim();
  const outcome = String(formData.get('outcome') ?? '').trim();
  const noteRaw = formData.get('note');
  const note =
    typeof noteRaw === 'string' && noteRaw.trim().length > 0 ? noteRaw.trim() : undefined;
  if (!id || !outcome) {
    revalidatePath(QUEUE_PATH);
    return;
  }
  try {
    await resolveLfgReport(session.token, id, { outcome, note });
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
