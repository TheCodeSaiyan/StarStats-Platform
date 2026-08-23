import Link from 'next/link';
import { ApiCallError, verifyEmail } from '@/lib/api';
import { logger } from '@/lib/logger';

export const metadata = { title: "Verify email" };

interface SearchParams {
  token?: string;
}





/**
 * Email verification landing page.
 *
 * The verification email links here with a `?token=…` query param.
 * The page is a server component so the API call (and the JWT for it,
 * if needed in the future) never reach the browser. There is no form
 * — the GET-from-link click *is* the action.
 *
 * Failure modes are coalesced behind a single message: an unknown
 * token, an expired token, and a network blip from the API all show
 * the same "request a new one" prompt. A future slice will wire a
 * resend endpoint; today the user has to sign in to trigger another
 * email manually.
 */
export default async function VerifyEmailPage(props: {
  searchParams: Promise<SearchParams>;
}) {
  const { token } = await props.searchParams;

  if (!token) {
    return (
      <div className="hp-authpage">
        <div className="hp-authcard">
          <span className="ss-eyebrow">Verify email</span>
          <h1>Missing verification token.</h1>
          <p className="hp-authsub">
            The verification link is incomplete. Open the email we sent you and
            click the link from there.
          </p>
          <Link href="/auth/login" className="ss-btn ss-btn--ghost">
            Back to sign in
          </Link>
        </div>
      </div>
    );
  }

  let outcome: 'verified' | 'invalid' = 'invalid';
  let claimedHandle: string | null = null;
  try {
    const resp = await verifyEmail({ token });
    if (resp.verified) {
      outcome = 'verified';
      claimedHandle = resp.claimed_handle;
    }
  } catch (e) {
    if (e instanceof ApiCallError && e.status === 400) {
      logger.info('verify rejected: invalid_or_expired');
    } else {
      logger.error({ err: e }, 'verify failed unexpectedly');
    }
    outcome = 'invalid';
  }

  if (outcome === 'verified') {
    return (
      <div className="hp-authpage">
        <div className="hp-authcard">
          <span className="ss-eyebrow">Email verified</span>
          <h1>Email verified.</h1>
          <p className="hp-authsub">
            Welcome aboard{claimedHandle ? `, ${claimedHandle}` : ''}. You can
            now sign in to your account.
          </p>
          <div className="ss-alert ss-alert--ok" role="status">
            Your account is ready to go.
          </div>
          <Link href="/auth/login" className="ss-btn ss-btn--primary">
            Sign in
          </Link>
        </div>
      </div>
    );
  }

  return (
    <div className="hp-authpage">
      <div className="hp-authcard">
        <span className="ss-eyebrow">Verify email</span>
        <h1>Token invalid or expired.</h1>
        <p className="hp-authsub">
          This verification link is no longer valid. Sign in to request a new
          one.
        </p>
        <Link href="/auth/login" className="ss-btn ss-btn--primary">
          Sign in
        </Link>
      </div>
    </div>
  );
}
