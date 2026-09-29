import { redirect } from 'next/navigation';
import type { NextRequest } from 'next/server';
import { ApiCallError, getMe, magicLinkRedeem } from '@/lib/api';
import { logger } from '@/lib/logger';
import { authAttemptsTotal } from '@/lib/metrics';
import { setSession } from '@/lib/session';
import { signInDestination } from '@/lib/sign-in-destination';

/**
 * Magic-link landing. The email link (and the tray's chat window, with
 * `next=/chat`) points here with `?token=...`.
 *
 * A route handler, not a page: it sets the session cookie, and Next 15
 * only lets a Server Action or Route Handler do that. As a page, every
 * redemption without two-factor threw "Cookies can only be modified in a
 * Server Action or Route Handler" and showed the error screen. The token
 * still never touches browser-side JS.
 *
 * If the account has TOTP enabled, the redeem returns an interim token +
 * `totp_required: true`; we forward to the same TOTP verify page the
 * password flow uses, keeping the second-factor surface uniform.
 */
export const dynamic = 'force-dynamic';

export async function GET(request: NextRequest): Promise<never> {
  const params = request.nextUrl.searchParams;
  const token = params.get('token');
  const destination = signInDestination(params.get('next'));

  if (!token) redirect('/auth/magic-link/redeem/invalid?reason=missing');

  let auth;
  try {
    auth = await magicLinkRedeem({ token });
  } catch (e) {
    if (e instanceof ApiCallError && e.status === 401) {
      authAttemptsTotal.inc({ action: 'magic_redeem', outcome: 'rejected' });
      logger.info('magic link redeem rejected: invalid_or_expired');
    } else {
      authAttemptsTotal.inc({ action: 'magic_redeem', outcome: 'unexpected' });
      logger.error({ err: e }, 'magic link redeem failed unexpectedly');
    }
    redirect('/auth/magic-link/redeem/invalid');
  }

  if (auth.totp_required) {
    authAttemptsTotal.inc({ action: 'magic_redeem', outcome: 'totp_required' });
    const carry = destination === '/me' ? '' : `&next=${encodeURIComponent(destination)}`;
    redirect(`/auth/totp-verify?interim=${encodeURIComponent(auth.token)}${carry}`);
  }

  let emailVerified = false;
  let staffRoles: string[] = [];
  try {
    const me = await getMe(auth.token);
    emailVerified = me.email_verified;
    staffRoles = me.staff_roles ?? [];
  } catch (meErr) {
    logger.warn(
      { err: meErr },
      'getMe after magic redeem failed; defaulting emailVerified=false',
    );
  }
  await setSession({
    token: auth.token,
    userId: auth.user_id,
    claimedHandle: auth.claimed_handle,
    emailVerified,
    staffRoles,
  });
  authAttemptsTotal.inc({ action: 'magic_redeem', outcome: 'success' });
  logger.info({ user_id: auth.user_id }, 'magic link redeem success');
  redirect(destination);
}
