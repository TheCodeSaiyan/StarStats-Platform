'use server';

import { revalidatePath } from 'next/cache';
import { getSession } from '@/lib/session';
import { setAdminAppearance, type AppearanceConfigApi } from '@/lib/api';
import { logger } from '@/lib/logger';

export type SaveAppearanceResult =
  | { ok: true; config: AppearanceConfigApi }
  | { ok: false; error: string };

/**
 * Save the sitewide appearance defaults (today: theme-switch wave
 * speed). Mirrors `saveWaitlistConfigAction` (`waitlist-admin.ts`):
 * echoes what the server actually stored, not what was sent.
 */
export async function saveAppearanceConfigAction(
  cfg: AppearanceConfigApi,
): Promise<SaveAppearanceResult> {
  const session = await getSession();
  if (!session) return { ok: false, error: 'not_authenticated' };
  try {
    const config = await setAdminAppearance(session.token, cfg);
    revalidatePath('/admin/appearance');
    return { ok: true, config };
  } catch (err) {
    logger.warn(
      { err, call: 'action.saveAppearanceConfig' },
      'save appearance config failed',
    );
    return { ok: false, error: 'save_failed' };
  }
}
