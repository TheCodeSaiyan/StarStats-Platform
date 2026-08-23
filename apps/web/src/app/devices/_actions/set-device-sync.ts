'use server';

import { revalidatePath } from 'next/cache';
import { redirect } from 'next/navigation';

import { ApiCallError, setDeviceSync } from '@/lib/api';
import { getSession } from '@/lib/session';
import { logger } from '@/lib/logger';

const LOGIN_NEXT = '/auth/login?next=/downloads';

/**
 * Server action that flips a single device's sync gate. Called from
 * the toggle <form> in the Uplinks group of `/downloads` (the Emitter);
 * it lived on `/devices` until that surface was folded in. Returns no value
 * — revalidatePath refreshes the list with the new state.
 *
 * Auth failures redirect to login (matching the sibling `revokeAction`
 * on the same page and every `/sharing` action) rather than throwing
 * — a throw surfaces the generic error boundary for what is just an
 * expired session (M-W15).
 */
export async function setUplinkSyncAction(formData: FormData): Promise<void> {
  const deviceId = String(formData.get('device_id') ?? '');
  const enabled = formData.get('enabled') === 'on';
  if (!deviceId) {
    redirect('/downloads?error=missing_id');
  }

  const session = await getSession();
  if (!session?.token) {
    redirect(LOGIN_NEXT);
  }

  try {
    await setDeviceSync(session.token, deviceId, enabled);
  } catch (e) {
    if (e instanceof ApiCallError && e.status === 401) {
      redirect(LOGIN_NEXT);
    }
    logger.warn({ err: e, deviceId, enabled }, 'setDeviceSync action failed');
    throw e;
  }
  revalidatePath('/downloads');
}
