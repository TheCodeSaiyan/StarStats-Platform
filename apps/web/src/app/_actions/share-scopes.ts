'use server';

import { revalidatePath } from 'next/cache';
import { getSession } from '@/lib/session';
import {
  updateMyShareScopes,
  type WidgetShareScopesApi,
} from '@/lib/api';
import { logger } from '@/lib/logger';

export type SaveShareScopesResult =
  | { ok: true; scopes: WidgetShareScopesApi }
  | { ok: false; error: string };

/**
 * Persist new per-widget sharing toggles for the current owner.
 *
 * Returns the server's canonical form (all five boolean fields).
 * Revalidates the owner's profile page so the next render sees the
 * updated visibility toggles.
 *
 * Mirrors the pattern established by `saveProfileLayoutAction`.
 */
export async function saveShareScopesAction(
  scopes: WidgetShareScopesApi,
): Promise<SaveShareScopesResult> {
  const session = await getSession();
  if (!session) {
    return { ok: false, error: 'not_authenticated' };
  }
  try {
    const saved = await updateMyShareScopes(session.token, scopes);
    revalidatePath(`/u/${session.claimedHandle}`);
    return { ok: true, scopes: saved };
  } catch (err) {
    logger.warn({ err, call: 'action.saveShareScopes' }, 'save share scopes failed');
    return { ok: false, error: 'save_failed' };
  }
}
