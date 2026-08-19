'use server';

import { getSession } from '@/lib/session';
import { putPreferences } from '@/lib/api';
import { isWaveSpeed, type WaveSpeed } from '@/lib/wave-speed';
import { logger } from '@/lib/logger';

/**
 * Persist a theme-wave-speed choice WITHOUT navigating. Mirrors
 * `persistThemeAction` (`app/_actions/theme.ts`): the Settings control
 * stamps `<html data-wave-speed>` client-side immediately (so the next
 * wave picks up the new duration in this tab) and calls this to write
 * it to `PUT /v1/me/preferences`.
 *
 * Silently no-ops for signed-out visitors — there's no per-user
 * preferences row to write to; they keep riding the sitewide
 * `appearance_config` default. Swallows backend failures like
 * `setTheme` does, so a transient API hiccup doesn't surface as a
 * broken control — the `<html>` attribute already reflects the choice
 * for this tab either way.
 */
export async function persistWaveSpeedAction(speed: WaveSpeed): Promise<void> {
  if (!isWaveSpeed(speed)) return;
  const session = await getSession();
  if (!session) return;
  try {
    await putPreferences(session.token, { theme_wave_speed: speed });
  } catch (e) {
    logger.warn({ err: e, speed }, 'persist wave speed failed');
  }
}
