'use server';

import { getSession } from '@/lib/session';
import { isTheme, setTheme, type Theme } from '@/lib/theme';

/**
 * Persist a theme choice WITHOUT navigating. The Settings swatch grid and
 * the TopBar toggle apply the theme client-side (with the wave) and then
 * call this to write the `ss-theme` cookie + `PUT /v1/me/preferences`.
 *
 * Deliberately has no `redirect`/`revalidatePath` — a redirect would reload
 * the page and kill the in-place animation. `setTheme` swallows backend
 * failures (the cookie still wins for this browser), so this resolves even
 * when the server is unreachable.
 */
export async function persistThemeAction(theme: Theme): Promise<void> {
  if (!isTheme(theme)) return;
  const session = await getSession();
  await setTheme(theme, session?.token);
}
