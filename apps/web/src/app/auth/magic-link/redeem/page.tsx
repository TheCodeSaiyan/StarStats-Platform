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

const mainStyle: React.CSSProperties = {
  maxWidth: 'none',
  padding: '32px 24px 60px',
  display: 'grid',
  placeItems: 'start center',
};

const cardStyle: React.CSSProperties = {
  width: '100%',
  maxWidth: 460,
  display: 'flex',
  flexDirection: 'column',
  gap: 20,
};

const titleStyle: React.CSSProperties = {
  margin: '8px 0 0',
  fontSize: 28,
  fontWeight: 600,
  letterSpacing: '-0.02em',
};

const subtitleStyle: React.CSSProperties = {
  margin: 0,
  color: 'var(--fg-muted)',
  fontSize: 14,
  lineHeight: 1.55,
};

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
      <main className="auth" style={mainStyle}>
        <div style={cardStyle}>
          <span className="ss-eyebrow">Sign-in link</span>
          <h1 style={titleStyle}>Missing sign-in token.</h1>
          <p style={subtitleStyle}>
            The link is incomplete. Open the email we sent you and click the
            link from there.
          </p>
          <Link href="/auth/magic-link" className="ss-btn ss-btn--primary">
            Request a new link
          </Link>
        </div>
      </main>
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
      <main className="auth" style={mainStyle}>
        <div style={cardStyle}>
          <span className="ss-eyebrow">Sign-in link</span>
          <h1 style={titleStyle}>Sign-in link invalid or expired.</h1>
          <p style={subtitleStyle}>
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
      </main>
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
