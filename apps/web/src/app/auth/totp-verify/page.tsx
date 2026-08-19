import Link from 'next/link';
import { redirect } from 'next/navigation';
import { ApiCallError, getMe, totpVerifyLogin } from '@/lib/api';
import { logger } from '@/lib/logger';
import { authAttemptsTotal } from '@/lib/metrics';
import { setSession } from '@/lib/session';

export const metadata = { title: "Two-factor auth" };

interface SearchParams {
  interim?: string;
  error?: string;
}

const mainStyle: React.CSSProperties = {
  maxWidth: 'none',
  padding: '32px 24px 60px',
  display: 'grid',
  placeItems: 'start center',
};

const cardStyle: React.CSSProperties = {
  width: '100%',
  maxWidth: 480,
  display: 'flex',
  flexDirection: 'column',
  gap: 22,
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

const formStyle: React.CSSProperties = {
  display: 'flex',
  flexDirection: 'column',
  gap: 16,
  margin: 0,
};

const otpRowStyle: React.CSSProperties = {
  justifyContent: 'flex-start',
};

const explainerStyle: React.CSSProperties = {
  padding: '12px 16px',
  background: 'var(--bg-elev)',
  border: '1px solid var(--border)',
  borderRadius: 'var(--r-sm)',
  color: 'var(--fg-dim)',
  fontSize: 12,
  lineHeight: 1.5,
};

/**
 * Second leg of the 2FA login flow.
 *
 * The login (or magic-link redeem) page redirects here with the
 * interim token in the URL when the account has TOTP enabled. We
 * collect the 6-digit code (or a recovery code) and trade the
 * interim token for a real session JWT.
 *
 * The interim token has a 5-minute TTL and is single-use — if the
 * user backs out of this page they have to re-enter their password.
 * That's by design: a leaked interim token without a 2FA code is
 * useless on its own.
 */
export default async function TotpVerifyPage(props: {
  searchParams: Promise<SearchParams>;
}) {
  const { interim, error } = await props.searchParams;

  if (!interim) {
    return (
      <main className="auth" style={mainStyle}>
        <div style={cardStyle}>
          <span className="ss-eyebrow">Two-factor verification</span>
          <h1 style={titleStyle}>Sign-in incomplete.</h1>
          <p style={subtitleStyle}>
            We don&apos;t have a half-finished sign-in for this browser. Start
            over from the sign-in page.
          </p>
          <Link href="/auth/login" className="ss-btn ss-btn--primary">
            Back to sign in
          </Link>
        </div>
      </main>
    );
  }

  async function action(formData: FormData) {
    'use server';
    const interimToken = String(formData.get('interim') ?? '');

    // The form ships either six single-digit cells (c0..c5) for the
    // TOTP path, or a recovery-code field. Concatenate the cells; if
    // empty, fall back to the recovery field. The API endpoint accepts
    // both shapes via the same `code` parameter.
    const cells = ['c0', 'c1', 'c2', 'c3', 'c4', 'c5']
      .map((k) => String(formData.get(k) ?? '').trim())
      .join('');
    const recovery = String(formData.get('recovery') ?? '').trim();
    const code = cells.length === 6 ? cells : recovery;

    let auth;
    try {
      auth = await totpVerifyLogin(interimToken, { code });
    } catch (e) {
      if (e instanceof ApiCallError) {
        if (e.status === 401) {
          authAttemptsTotal.inc({
            action: 'totp_verify',
            outcome: 'rejected',
          });
          logger.info('totp verify-login rejected');
          redirect(
            `/auth/totp-verify?interim=${encodeURIComponent(interimToken)}&error=invalid`,
          );
        }
        if (e.status === 403) {
          // Bearer is not a LoginInterim token (or the token is
          // missing/garbled). Send them back to start.
          redirect('/auth/login?error=interim_required');
        }
      }
      authAttemptsTotal.inc({ action: 'totp_verify', outcome: 'unexpected' });
      logger.error({ err: e }, 'totp verify-login failed unexpectedly');
      redirect(
        `/auth/totp-verify?interim=${encodeURIComponent(interimToken)}&error=unexpected`,
      );
    }

    // Hydrate emailVerified — same as the password-login success path.
    let emailVerified = false;
    let staffRoles: string[] = [];
    try {
      const me = await getMe(auth.token);
      emailVerified = me.email_verified;
      staffRoles = me.staff_roles ?? [];
    } catch (meErr) {
      logger.warn(
        { err: meErr },
        'getMe after totp verify failed; defaulting emailVerified=false',
      );
    }
    await setSession({
      token: auth.token,
      userId: auth.user_id,
      claimedHandle: auth.claimed_handle,
      emailVerified,
      staffRoles,
    });
    authAttemptsTotal.inc({ action: 'totp_verify', outcome: 'success' });
    logger.info({ user_id: auth.user_id }, 'totp verify success');
    redirect('/me');
  }

  return (
    <main className="auth" style={mainStyle}>
      <div style={cardStyle}>
        <span className="ss-eyebrow">Two-factor verification</span>
        <h1 style={titleStyle}>Authentication code.</h1>
        <p style={subtitleStyle}>
          Open your authenticator app and type the 6-digit code for StarStats.
          Codes refresh every 30 seconds.
        </p>

        {error === 'invalid' && (
          <div className="ss-alert ss-alert--danger" role="alert">
            That code didn&apos;t match. Check the time on your device and try
            again.
          </div>
        )}
        {error === 'unexpected' && (
          <div className="ss-alert ss-alert--danger" role="alert">
            Something went wrong. Please try again.
          </div>
        )}

        <form action={action} style={formStyle}>
          <input type="hidden" name="interim" value={interim} />

          <div className="ss-label">
            <span className="ss-label-text">Code</span>
            <div className="ss-otp" style={otpRowStyle}>
              <input
                className="ss-otp-cell"
                name="c0"
                maxLength={1}
                inputMode="numeric"
                autoComplete="one-time-code"
                pattern="[0-9]"
                aria-label="Digit 1"
              />
              <input
                className="ss-otp-cell"
                name="c1"
                maxLength={1}
                inputMode="numeric"
                pattern="[0-9]"
                aria-label="Digit 2"
              />
              <input
                className="ss-otp-cell"
                name="c2"
                maxLength={1}
                inputMode="numeric"
                pattern="[0-9]"
                aria-label="Digit 3"
              />
              <span className="ss-otp-sep" aria-hidden="true" />
              <input
                className="ss-otp-cell"
                name="c3"
                maxLength={1}
                inputMode="numeric"
                pattern="[0-9]"
                aria-label="Digit 4"
              />
              <input
                className="ss-otp-cell"
                name="c4"
                maxLength={1}
                inputMode="numeric"
                pattern="[0-9]"
                aria-label="Digit 5"
              />
              <input
                className="ss-otp-cell"
                name="c5"
                maxLength={1}
                inputMode="numeric"
                pattern="[0-9]"
                aria-label="Digit 6"
              />
            </div>
          </div>

          <details>
            <summary
              style={{
                cursor: 'pointer',
                color: 'var(--fg-muted)',
                fontSize: 13,
                marginBottom: 8,
              }}
            >
              Lost your authenticator? Use a recovery code
            </summary>
            <label className="ss-label" style={{ marginTop: 8 }}>
              <span className="ss-label-text">Recovery code</span>
              <input
                className="ss-input mono"
                type="text"
                name="recovery"
                inputMode="text"
                autoComplete="off"
                spellCheck={false}
                placeholder="XXXX-XXXX-XXXX-XXXX"
              />
              <small style={{ color: 'var(--fg-dim)', fontSize: 12 }}>
                Each recovery code is single-use.
              </small>
            </label>
          </details>

          <div
            style={{
              display: 'flex',
              gap: 8,
              justifyContent: 'space-between',
              flexWrap: 'wrap',
            }}
          >
            <Link href="/auth/login" className="ss-btn ss-btn--ghost">
              Back
            </Link>
            <button type="submit" className="ss-btn ss-btn--primary">
              Verify &amp; sign in
            </button>
          </div>
        </form>

        <div style={explainerStyle}>
          <strong style={{ color: 'var(--fg-muted)' }}>Why this exists.</strong>{' '}
          Your interim sign-in token is single-use and expires in 5 minutes.
          Backing out invalidates it — that&apos;s by design.
        </div>
      </div>
    </main>
  );
}
