import Link from 'next/link';
import { redirect } from 'next/navigation';
import { passwordResetStart } from '@/lib/api';
import { logger } from '@/lib/logger';

export const metadata = { title: "Reset password" };

interface SearchParams {
  sent?: string;
  error?: string;
}







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
      <div className="hp-authpage">
        <div className="hp-authcard">
          <span className="ss-eyebrow">Reset link sent</span>
          <h1>Check your email.</h1>
          <p className="hp-authsub">
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
      </div>
    );
  }

  return (
    <div className="hp-authpage">
      <div className="hp-authcard">
        <span className="ss-eyebrow">Reset password</span>
        <h1>Forgot your password?</h1>
        <p className="hp-authsub">
          Enter the email on your account and we&apos;ll send a link to
          choose a new password.
        </p>

        {error === '1' && (
          <div className="ss-alert ss-alert--danger" role="alert">
            Something went wrong. Please try again.
          </div>
        )}

        <form action={action} className="hp-authform">
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
          <div className="hp-authactions">
            <button type="submit" className="ss-btn ss-btn--primary">
              Send reset link
            </button>
            <Link href="/auth/login" className="ss-btn ss-btn--ghost">
              Back to sign in
            </Link>
          </div>
        </form>
      </div>
    </div>
  );
}
