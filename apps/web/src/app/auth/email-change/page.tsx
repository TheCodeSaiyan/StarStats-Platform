import Link from 'next/link';
import { ApiCallError, emailChangeVerify } from '@/lib/api';
import { logger } from '@/lib/logger';

export const metadata = { title: "Confirm email change" };

interface SearchParams {
  token?: string;
}





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
      <div className="hp-authpage">
        <div className="hp-authcard">
          <span className="ss-eyebrow">Email change</span>
          <h1>Missing confirmation token.</h1>
          <p className="hp-authsub">
            The confirmation link is incomplete. Open the email we sent to your
            new email and click the link from there.
          </p>
          <Link href="/settings" className="ss-btn ss-btn--ghost">
            Back to account settings
          </Link>
        </div>
      </div>
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
      <div className="hp-authpage">
        <div className="hp-authcard">
          <span className="ss-eyebrow">Email updated</span>
          <h1>Email updated.</h1>
          <p className="hp-authsub">
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
      </div>
    );
  }

  if (outcome === 'taken') {
    return (
      <div className="hp-authpage">
        <div className="hp-authcard">
          <span className="ss-eyebrow">Email change</span>
          <h1>Address already in use.</h1>
          <p className="hp-authsub">
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
      </div>
    );
  }

  return (
    <div className="hp-authpage">
      <div className="hp-authcard">
        <span className="ss-eyebrow">Email change</span>
        <h1>Token invalid or expired.</h1>
        <p className="hp-authsub">
          This confirmation link is no longer valid. Start the email change
          again from your account settings.
        </p>
        <Link href="/settings" className="ss-btn ss-btn--primary">
          Back to account settings
        </Link>
      </div>
    </div>
  );
}
