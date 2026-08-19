import { NextResponse } from 'next/server';
import { clearSession } from '@/lib/session';

/**
 * GET /auth/logout — clear the session cookie and bounce to home.
 *
 * GET (rather than POST) is deliberate: the only side effect is
 * "delete *your own* cookie" and supporting plain anchor tags
 * keeps the no-JS path working.
 *
 * Why a manual 302 with a relative `Location: /` instead of
 * `NextResponse.redirect(new URL('/', req.url))`: inside the container
 * `req.url` is `http://0.0.0.0:3000/auth/logout` (Next's internal
 * bind), so `new URL('/', req.url)` produces `http://0.0.0.0:3000/`
 * and the reverse proxy upgrades the scheme to `https`, sending the
 * user to a broken `https://0.0.0.0:3000/`. Per RFC 7231 §7.1.2 a
 * relative `Location` is resolved by the browser against the
 * effective request URI — i.e. whatever the user actually typed —
 * which is exactly what we want regardless of proxy config.
 */
export async function GET() {
  await clearSession();
  return new NextResponse(null, {
    status: 302,
    headers: { Location: '/' },
  });
}
