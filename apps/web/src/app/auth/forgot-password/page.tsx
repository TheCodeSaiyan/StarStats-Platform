import Link from 'next/link';
import { redirect } from 'next/navigation';
import { passwordResetStart } from '@/lib/api';
import { logger } from '@/lib/logger';

export const metadata = { title: "Reset password" };

interface SearchParams {
  sent?: string;
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
  maxWidth: 420,
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

const formStyle: React.CSSProperties = {
  display: 'flex',
  flexDirection: 'column',
  gap: 14,
  margin: 0,
};

const actionsStyle: React.CSSProperties = {
  display: 'flex',
  flexDirection: 'column',
  gap: 8,
  marginTop: 4,
};

/**
 * Password-reset start page.
 *
 * Submits an email; the server *always* returns 200 even if the
 * address isn't on file (anti-enumeration). Redirect to ?sent=1 on
 * success so the user sees the confirmation copy regardless. Any
 * other error path (network blip, server 5xx) lands at ?error=1
 * with a generic prompt to try again.
 */
export default async function ForgotPasswordPage(props: {
  searchParams: Promise<SearchParams>;
}) {
  const { sent, error } = await props.searchParams;

  async function action(formData: FormData) {
    'use server';
    const email = String(formData.get('email') ?? '').trim();
    try {
      await passwordResetStart({ email });
      logger.info('password reset start requested');
    } catch (e) {
      logger.error({ err: e }, 'password reset start failed unexpectedly');
      redirect('/auth/forgot-password?error=1');
    }
    redirect('/auth/forgot-password?sent=1');
  }

  if (sent === '1') {
    return (
      <main className="auth" style={mainStyle}>
        <div style={cardStyle}>
          <span className="ss-eyebrow">Reset link sent</span>
          <h1 style={titleStyle}>Check your email.</h1>
          <p style={subtitleStyle}>
            If an account exists for that address, we&apos;ve sent a
            password-reset link. The link expires in 30 minutes.
          </p>
          <div className="ss-alert" style={{ alignItems: 'flex-start' }}>
            <span style={{ color: 'var(--fg-muted)' }}>
              Didn&apos;t arrive? Double-check the spelling and{' '}
              <Link href="/auth/forgot-password" style={{ color: 'var(--accent)' }}>
                try again
              </Link>
              .
            </span>
          </div>
          <Link href="/auth/login" className="ss-btn ss-btn--ghost">
            Back to sign in
          </Link>
        </div>
      </main>
    );
  }

  return (
    <main className="auth" style={mainStyle}>
      <div style={cardStyle}>
        <span className="ss-eyebrow">Reset password</span>
        <h1 style={titleStyle}>Forgot your password?</h1>
        <p style={subtitleStyle}>
          Enter the email on your account and we&apos;ll send a link to
          choose a new password.
        </p>

        {error === '1' && (
          <div className="ss-alert ss-alert--danger" role="alert">
            Something went wrong. Please try again.
          </div>
        )}

        <form action={action} style={formStyle}>
          <label className="ss-label">
            <span className="ss-label-text">Email</span>
            <input
              className="ss-input"
              type="email"
              name="email"
              required
              autoComplete="email"
              spellCheck={false}
              placeholder="you@example.com"
            />
          </label>
          <div style={actionsStyle}>
            <button type="submit" className="ss-btn ss-btn--primary">
              Send reset link
            </button>
            <Link href="/auth/login" className="ss-btn ss-btn--ghost">
              Back to sign in
            </Link>
          </div>
        </form>
      </div>
    </main>
  );
}
