import Link from 'next/link';
import { redirect } from 'next/navigation';
import { ApiCallError, passwordResetComplete } from '@/lib/api';
import { logger } from '@/lib/logger';

export const metadata = { title: "Set new password" };

interface SearchParams {
  token?: string;
  error?: string;
  done?: string;
}






/**
 * Password-reset completion page.
 *
 * The reset email links here with `?token=…`. We render a form that
 * captures the new password and POSTs to the API; on success all
 * device tokens for the user are revoked server-side and we redirect
 * to ?done=1 with a "sign in" prompt.
 *
 * Token validation happens server-side at submit time, not on page
 * load, so a stale link doesn't burn until the user actually attempts
 * the change. 400/401 collapse to the same "link invalid or expired"
 * copy — same disclosure surface as the verify page.
 */
export default async function ResetPasswordPage(props: {
  searchParams: Promise<SearchParams>;
}) {
  const { token, error, done } = await props.searchParams;

  if (done === '1') {
    return (
      <div className="hp-authpage">
        <div className="hp-authcard">
          <span className="ss-eyebrow">Password updated</span>
          <h1>Password updated.</h1>
          <p className="hp-authsub">
            Your password has been changed. For safety we&apos;ve signed out
            every paired device — sign in again to continue.
          </p>
          <Link href="/auth/login" className="ss-btn ss-btn--primary">
            Sign in
          </Link>
        </div>
      </div>
    );
  }

  if (!token) {
    return (
      <div className="hp-authpage">
        <div className="hp-authcard">
          <span className="ss-eyebrow">Reset password</span>
          <h1>Missing reset token.</h1>
          <p className="hp-authsub">
            The reset link is incomplete. Open the email we sent you and click
            the link from there.
          </p>
          <Link href="/auth/forgot-password" className="ss-btn ss-btn--primary">
            Request a new link
          </Link>
        </div>
      </div>
    );
  }

  async function action(formData: FormData) {
    'use server';
    const submittedToken = String(formData.get('token') ?? '');
    const newPassword = String(formData.get('new_password') ?? '');
    const confirmPassword = String(formData.get('confirm_password') ?? '');

    if (newPassword !== confirmPassword) {
      redirect(
        `/auth/reset-password?token=${encodeURIComponent(submittedToken)}&error=mismatch`,
      );
    }
    if (newPassword.length < 12) {
      redirect(
        `/auth/reset-password?token=${encodeURIComponent(submittedToken)}&error=weak`,
      );
    }

    try {
      await passwordResetComplete({
        token: submittedToken,
        new_password: newPassword,
      });
      logger.info('password reset complete success');
    } catch (e) {
      if (
        e instanceof ApiCallError &&
        (e.status === 400 || e.status === 401)
      ) {
        logger.info('password reset rejected: invalid_or_expired');
        redirect('/auth/reset-password?error=invalid');
      }
      logger.error({ err: e }, 'password reset failed unexpectedly');
      redirect(
        `/auth/reset-password?token=${encodeURIComponent(submittedToken)}&error=unexpected`,
      );
    }
    redirect('/auth/reset-password?done=1');
  }

  return (
    <div className="hp-authpage">
      <div className="hp-authcard">
        <span className="ss-eyebrow">Reset password</span>
        <h1>Choose a new password.</h1>
        <p className="hp-authsub">
          Pick something at least 12 characters. All your paired devices will
          be signed out when you save.
        </p>

        {error === 'invalid' && (
          <div className="ss-alert ss-alert--danger" role="alert">
            This reset link is no longer valid. Request a new one below.
          </div>
        )}
        {error === 'mismatch' && (
          <div className="ss-alert ss-alert--danger" role="alert">
            The two passwords don&apos;t match.
          </div>
        )}
        {error === 'weak' && (
          <div className="ss-alert ss-alert--danger" role="alert">
            Password must be at least 12 characters.
          </div>
        )}
        {error === 'unexpected' && (
          <div className="ss-alert ss-alert--danger" role="alert">
            Something went wrong. Please try again.
          </div>
        )}

        <form action={action} className="hp-authform">
          <input type="hidden" name="token" value={token} />
          <label className="ss-label">
            <span className="ss-label-text">New password</span>
            <input
              className="ss-input"
              type="password"
              name="new_password"
              required
              minLength={12}
              autoComplete="new-password"
            />
          </label>
          <label className="ss-label">
            <span className="ss-label-text">Confirm new password</span>
            <input
              className="ss-input"
              type="password"
              name="confirm_password"
              required
              minLength={12}
              autoComplete="new-password"
            />
          </label>
          <button
            type="submit"
            className="ss-btn ss-btn--primary"
            style={{ marginTop: 4 }}
          >
            Set new password
          </button>
        </form>

        <hr className="ss-rule" />

        <p style={{ margin: 0, fontSize: 13, color: 'var(--fg-muted)' }}>
          Need a fresh link?{' '}
          <Link href="/auth/forgot-password" style={{ color: 'var(--accent)' }}>
            Request one
          </Link>
          .
        </p>
      </div>
    </div>
  );
}
