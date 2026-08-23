import 'server-only';
import { cookies } from 'next/headers';

import { getPreferences, putPreferences } from '@/lib/api';
import { logger } from '@/lib/logger';

export type Theme = 'stanton' | 'pyro' | 'terra' | 'nyx';

export const THEMES: readonly Theme[] = ['stanton', 'pyro', 'terra', 'nyx'];
// Gap C: the projection is drawn in terra (cyan) — every guideline card, UI kit
// screen and screenshot in the design system uses it, and it is the calibration
// the volume was composed against. Readers who picked a calibration keep it;
// only those who never chose see the change.
export const DEFAULT_THEME: Theme = 'terra';
export const THEME_COOKIE = 'ss-theme';

const VALID = new Set<Theme>(THEMES);

/** ~ 1 year. Long enough that "set once" actually sticks across sessions. */
const THEME_COOKIE_MAX_AGE = 60 * 60 * 24 * 365;

export function isTheme(value: unknown): value is Theme {
  return typeof value === 'string' && VALID.has(value as Theme);
}

/**
 * Resolve the user's theme.
 *
 * - Anonymous visitors (no bearer): the `ss-theme` cookie is the
 *   only source of truth.
 * - Authenticated visitors: reconcile with the server. If the
 *   server has a theme that differs from the cookie, the server
 *   wins. The cookie is NOT updated here — Next.js 15 disallows
 *   cookie writes in pure rendering paths (server components). The
 *   cookie will catch up on the next explicit `setTheme` call
 *   (which runs inside a server action where mutations are allowed).
 *   If the server call fails or returns no theme, the cookie wins
 *   (degrade quietly).
 *
 * Called from the root layout on every server-rendered page. The
 * `/v1/me/preferences` route is per-IP rate-limited (1/s sustained,
 * burst 5) — well above realistic page-navigation pace, so calling
 * on every render is fine.
 */
export async function getTheme(bearer?: string): Promise<Theme> {
  const store = await cookies();
  const cookieValue = store.get(THEME_COOKIE)?.value;
  const cookieTheme = isTheme(cookieValue) ? (cookieValue as Theme) : DEFAULT_THEME;

  if (!bearer) {
    return cookieTheme;
  }

  try {
    const prefs = await getPreferences(bearer);
    if (prefs.theme && isTheme(prefs.theme) && prefs.theme !== cookieTheme) {
      return prefs.theme as Theme;
    }
  } catch (e) {
    logger.warn({ err: e }, 'get preferences failed during getTheme; falling back to cookie');
  }
  return cookieTheme;
}

/**
 * Persist the user's theme choice. Sets the local `ss-theme` cookie so
 * SSR's `<html data-theme>` reflects the choice on the next render, then
 * pushes the same value to the server-side preferences row so it follows
 * the user across devices.
 *
 * The local cookie is the source of truth for paint -- if the server PUT
 * fails (network blip, 500, transient backend issue), we still want the UI
 * to honour the user's choice in this browser. The error is logged but
 * swallowed so the calling server action can complete cleanly.
 *
 * The optional `bearer` arg lets the caller forward an existing session
 * token; when omitted, the server-side persistence step is skipped (used
 * by unauthenticated flows that only need the local cookie).
 */
export async function setTheme(
  theme: Theme,
  bearer?: string,
): Promise<void> {
  if (!isTheme(theme)) {
    throw new Error(`invalid theme: ${String(theme)}`);
  }

  const store = await cookies();
  store.set(THEME_COOKIE, theme, {
    httpOnly: false,
    sameSite: 'lax',
    secure: process.env.NODE_ENV === 'production',
    path: '/',
    maxAge: THEME_COOKIE_MAX_AGE,
  });

  if (bearer) {
    try {
      await putPreferences(bearer, { theme });
    } catch (e) {
      // Cookie still wins -- degrade quietly so the local UX stays
      // consistent even when the backend is misbehaving.
      logger.warn({ err: e, theme }, 'put preferences failed');
    }
  }
}
