'use server';

/**
 * Dashboard-card refresh actions. Audit v2 §08 promoted the
 * "Refresh from Settings" link on each RSI snapshot card into an
 * inline button so the user can pull a fresh snapshot without
 * leaving /dashboard. Each action mirrors the existing /settings
 * helpers in `apps/web/src/app/settings/page.tsx` — same error
 * mapping, same login redirect — but revalidates /dashboard
 * instead so the new snapshot lands in the card on the next render.
 *
 * Hangar has no equivalent server endpoint (the tray client is the
 * only writer); HangarCard keeps the settings-link CTA documented
 * with a TODO referencing this comment.
 */

import { revalidatePath } from 'next/cache';
import { redirect } from 'next/navigation';
import {
  ApiCallError,
  refreshProfile,
  refreshRsiOrgs,
} from '@/lib/api';
import { getSession } from '@/lib/session';

const LOGIN_NEXT = '/auth/login?next=/me';

export async function refreshProfileAction(): Promise<void> {
  const session = await getSession();
  if (!session) redirect(LOGIN_NEXT);
  try {
    await refreshProfile(session.token);
  } catch (e) {
    // 401 → re-auth; 422 → handle not yet verified (send the user to
    // settings where the verify flow lives). Any other ApiCallError
    // falls through to /dashboard — the snapshot card itself surfaces
    // the stale state on the next render.
    if (e instanceof ApiCallError) {
      if (e.status === 401) redirect(LOGIN_NEXT);
      if (e.status === 422) {
        redirect('/settings?error=rsi_handle_not_verified#rsi');
      }
    }
    // Fall through: revalidate to clear any stale render, then return.
  }
  revalidatePath('/me');
}

export async function refreshOrgsAction(): Promise<void> {
  const session = await getSession();
  if (!session) redirect(LOGIN_NEXT);
  try {
    await refreshRsiOrgs(session.token);
  } catch (e) {
    if (e instanceof ApiCallError) {
      if (e.status === 401) redirect(LOGIN_NEXT);
      if (e.status === 422) {
        redirect('/settings?error=rsi_handle_not_verified#rsi');
      }
    }
  }
  revalidatePath('/me');
}
