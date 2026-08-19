/**
 * Session cookie management. The cookie holds the user JWT minted
 * by the StarStats API; we store it HttpOnly so client-side JS
 * can't reach it (mitigates XSS exfil) and SameSite=Lax (allows
 * top-level navigation but blocks cross-site form-post CSRF).
 *
 * Server-only — uses Next.js cookies() which is unavailable in the
 * browser bundle.
 */

import 'server-only';
import { cookies } from 'next/headers';

const COOKIE_NAME = 'starstats_session';
const SESSION_TTL_SECS = 60 * 60; // 1 h, matches the API's user TTL.

export interface Session {
  token: string;
  userId: string;
  claimedHandle: string;
  /**
   * Whether the user has verified the email tied to this account.
   * Captured at login/signup so the layout can render an unverified
   * banner without an extra API hop on every request.
   *
   * Legacy cookies minted before this field existed default to
   * `false` on read — degraded mode where the banner shows even if
   * the server-side state is verified.
   */
  emailVerified: boolean;
  /**
   * Site-wide staff grants the user holds (e.g. `["moderator"]`,
   * `["admin"]`). Captured from `/v1/auth/me` at session-mint time so
   * `/admin` gating doesn't need an extra API call per page nav. Admin
   * implies moderator on the server side, so role checks here can use
   * `.some(r => r === 'admin' || r === 'moderator')`.
   *
   * Legacy cookies minted before this field existed default to `[]`
   * on read — degraded mode where staff users lose admin access until
   * they re-authenticate.
   */
  staffRoles: string[];
}

interface SerialisedSession {
  t: string;
  u: string;
  h: string;
  /** email_verified — optional in JSON for backwards compat. */
  v?: boolean;
  /** staff_roles — optional in JSON for backwards compat. */
  r?: string[];
}

export async function setSession(session: Session): Promise<void> {
  const jar = await cookies();
  const value: SerialisedSession = {
    t: session.token,
    u: session.userId,
    h: session.claimedHandle,
    v: session.emailVerified,
    r: session.staffRoles,
  };
  jar.set({
    name: COOKIE_NAME,
    value: JSON.stringify(value),
    httpOnly: true,
    secure: process.env.NODE_ENV === 'production',
    sameSite: 'lax',
    path: '/',
    maxAge: SESSION_TTL_SECS,
  });
}

export async function getSession(): Promise<Session | null> {
  const jar = await cookies();
  const raw = jar.get(COOKIE_NAME)?.value;
  if (!raw) return null;
  try {
    const parsed = JSON.parse(raw) as SerialisedSession;
    return {
      token: parsed.t,
      userId: parsed.u,
      claimedHandle: parsed.h,
      emailVerified: parsed.v ?? false,
      staffRoles: parsed.r ?? [],
    };
  } catch {
    // Tampered or stale cookie — treat as no session.
    return null;
  }
}

export async function clearSession(): Promise<void> {
  const jar = await cookies();
  jar.delete(COOKIE_NAME);
}
