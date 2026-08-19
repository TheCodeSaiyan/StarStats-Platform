'use server';

import { getSession } from '@/lib/session';
import { putPreferences, type UserPreferences } from '@/lib/api';
import { logger } from '@/lib/logger';

/**
 * Persist KB view-config changes to the signed-in user's profile.
 * Best-effort: logs + swallows on failure so the client's local
 * (localStorage-mirrored) choice still stands. No-op when not signed in.
 */
export async function saveKbPrefs(partial: Partial<UserPreferences>): Promise<void> {
  const session = await getSession();
  if (!session?.token) return;
  try {
    await putPreferences(session.token, partial as UserPreferences);
  } catch (e) {
    logger.warn({ err: e }, 'saveKbPrefs failed');
  }
}
