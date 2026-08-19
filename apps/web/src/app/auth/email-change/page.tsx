import Link from 'next/link';
import { ApiCallError, emailChangeVerify } from '@/lib/api';
import { logger } from '@/lib/logger';

export const metadata = { title: "Confirm email change" };

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
 * Email-change confirmation landing page.
 *
 * The confirmation email (sent to the *new* address) links here with
 * `?token=…`. The endpoint is unauthenticated — possession of the
 * token is the auth — so the click itself completes the swap. There
 * is no form: it's a one-shot landing.
 *
 * 400/401 means token is unknown or expired; 409 means the new
 * address has been claimed by someone else in the meantime. We
 * surface them as distinct copy because the recovery path differs
 * (request a new link vs pick a different address).
 */
export default async function EmailChangeVerifyPage(props: {
  searchParams: Promise<SearchParams>;
}) {
  const { token } = await props.searchParams;

  if (!token) {
    return (
      <main className="auth" style={mainStyle}>
        <div style={cardStyle}>
          <span className="ss-eyebrow">Email change</span>
          <h1 style={titleStyle}>Missing confirmation token.</h1>
          <p style={subtitleStyle}>
            The confirmation link is incomplete. Open the email we sent to your
            new email and click the link from there.
          </p>
          <Link href="/settings" className="ss-btn ss-btn--ghost">
            Back to account settings
          </Link>
        </div>
      </main>
    );
  }

  let outcome: 'changed' | 'invalid' | 'taken' = 'invalid';
  let newEmail: string | null = null;
  try {
    const resp = await emailChangeVerify({ token });
    outcome = 'changed';
    newEmail = resp.email;
  } catch (e) {
    if (e instanceof ApiCallError && e.status === 409) {
      outcome = 'taken';
      logger.info('email change rejected: address taken');
    } else if (
      e instanceof ApiCallError &&
      (e.status === 400 || e.status === 401)
    ) {
      logger.info('email change rejected: invalid_or_expired');
      outcome = 'invalid';
    } else {
      logger.error({ err: e }, 'email change verify failed unexpectedly');
      outcome = 'invalid';
    }
  }

  if (outcome === 'changed') {
    return (
      <main className="auth" style={mainStyle}>
        <div style={cardStyle}>
          <span className="ss-eyebrow">Email updated</span>
          <h1 style={titleStyle}>Email updated.</h1>
          <p style={subtitleStyle}>
            Your sign-in email is now{' '}
            <strong className="mono" style={{ color: 'var(--fg)' }}>
              {newEmail ?? 'updated'}
            </strong>
            . Use it the next time you sign in.
          </p>
          <div className="ss-alert ss-alert--ok" role="status">
            Change confirmed.
          </div>
          <Link href="/settings" className="ss-btn ss-btn--primary">
            Back to account settings
          </Link>
        </div>
      </main>
    );
  }

  if (outcome === 'taken') {
    return (
      <main className="auth" style={mainStyle}>
        <div style={cardStyle}>
          <span className="ss-eyebrow">Email change</span>
          <h1 style={titleStyle}>Address already in use.</h1>
          <p style={subtitleStyle}>
            Someone else claimed that email while your confirmation was
            pending. Pick a different address and try again from your settings
            page.
          </p>
          <div className="ss-alert ss-alert--warn" role="alert">
            No change was made to your account.
          </div>
          <Link href="/settings" className="ss-btn ss-btn--primary">
            Back to account settings
          </Link>
        </div>
      </main>
    );
  }

  return (
    <main className="auth" style={mainStyle}>
      <div style={cardStyle}>
        <span className="ss-eyebrow">Email change</span>
        <h1 style={titleStyle}>Token invalid or expired.</h1>
        <p style={subtitleStyle}>
          This confirmation link is no longer valid. Start the email change
          again from your account settings.
        </p>
        <Link href="/settings" className="ss-btn ss-btn--primary">
          Back to account settings
        </Link>
      </div>
    </main>
  );
}
