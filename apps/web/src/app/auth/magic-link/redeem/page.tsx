import Link from 'next/link';
import { redirect } from 'next/navigation';
import { ApiCallError, getMe, magicLinkRedeem } from '@/lib/api';
import { logger } from '@/lib/logger';
import { authAttemptsTotal } from '@/lib/metrics';
import { setSession } from '@/lib/session';

export const metadata = { title: "Magic link" };

interface SearchParams {
  token?: string;
}





/**
 * Magic-link landing page.
 *
 * The email link points here with `?token=...`. The page is a
 * server component so the redemption + session cookie set happen
 * in one round trip without the token ever touching browser-side JS.
 *
 * If the account has TOTP enabled, the redeem returns an interim
 * token + `totp_required: true`; we forward to the same TOTP verify
 * page the password flow uses, keeping the second-factor surface
 * uniform.
 */
export default async function MagicLinkRedeemPage(props: {
  searchParams: Promise<SearchParams>;
}) {
  const { token } = await props.searchParams;

  if (!token) {
    return (
      <div className="hp-authpage">
        <div className="hp-authcard">
          <span className="ss-eyebrow">Sign-in link</span>
          <h1>Missing sign-in token.</h1>
          <p className="hp-authsub">
            The link is incomplete. Open the email we sent you and click the
            link from there.
          </p>
          <Link href="/auth/magic-link" className="ss-btn ss-btn--primary">
            Request a new link
          </Link>
        </div>
      </div>
    );
  }

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
    return (
      <div className="hp-authpage">
        <div className="hp-authcard">
          <span className="ss-eyebrow">Sign-in link</span>
          <h1>Sign-in link invalid or expired.</h1>
          <p className="hp-authsub">
            This link can&apos;t be used. It may have expired (links are good
            for 15 minutes), already been clicked, or never have been issued.
            Request a new one to try again.
          </p>
          <div className="ss-alert ss-alert--warn" role="alert">
            Old links are invalidated automatically when a newer one is
            requested.
          </div>
          <Link href="/auth/magic-link" className="ss-btn ss-btn--primary">
            Request a new link
          </Link>
        </div>
      </div>
    );
  }

  if (auth.totp_required) {
    authAttemptsTotal.inc({ action: 'magic_redeem', outcome: 'totp_required' });
    redirect(
      `/auth/totp-verify?interim=${encodeURIComponent(auth.token)}`,
    );
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
  redirect('/me');
}
