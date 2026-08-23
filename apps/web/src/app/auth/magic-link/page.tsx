import Link from 'next/link';
import { redirect } from 'next/navigation';
import { magicLinkStart } from '@/lib/api';
import { logger } from '@/lib/logger';

export const metadata = { title: "Magic link" };

interface SearchParams {
  sent?: string;
  error?: string;
}

/**
 * Magic-link request page.
 *
 * Anti-enumeration: the server always returns 200, so we always
 * land on the "check your inbox" message regardless of whether the
 * email maps to an account.
 */
export default async function MagicLinkPage(props: {
  searchParams: Promise<SearchParams>;
}) {
  const { sent, error } = await props.searchParams;

  async function action(formData: FormData) {
    'use server';
    const email = String(formData.get('email') ?? '').trim();
    try {
      await magicLinkStart({ email });
    } catch (e) {
      logger.error({ err: e }, 'magic link start failed unexpectedly');
      redirect('/auth/magic-link?error=1');
    }
    redirect('/auth/magic-link?sent=1');
  }

  if (sent === '1') {
    return (
      <div className="hp-authpage">
        <div className="hp-authcard">
          <div className="hp-authmark" aria-hidden="true">
            ↗
          </div>
          <span className="ss-eyebrow">One-time link sent</span>
          <h1>Check your email.</h1>
          <p className="hp-authsub">
            If an account exists for that address, we&apos;ve sent a one-shot
            sign-in link. The link expires in 15 minutes and works exactly
            once.
          </p>

          <div className="ss-alert" style={{ alignItems: 'flex-start' }}>
            <span style={{ color: 'var(--fg-muted)' }}>
              Didn&apos;t arrive? Check spam, or wait 30 seconds and{' '}
              <Link href="/auth/magic-link" style={{ color: 'var(--accent)' }}>
                request another
              </Link>
              . Old links are invalidated automatically.
            </span>
          </div>

          <div className="hp-authactions">
            <Link href="/auth/magic-link" className="ss-btn ss-btn--primary">
              Resend link
            </Link>
            <Link href="/auth/login" className="ss-btn ss-btn--ghost">
              Back to sign in
            </Link>
          </div>
        </div>
      </div>
    );
  }

  return (
    <div className="hp-authpage">
      <div className="hp-authcard">
        <span className="ss-eyebrow">Magic link</span>
        <h1>Sign in with a one-time link.</h1>
        <p className="hp-authsub">
          Skip the password — we&apos;ll send a link to your email that
          signs you in for one session.
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
              Send magic link to my email
            </button>
            <Link href="/auth/login" className="ss-btn ss-btn--ghost">
              Use password instead
            </Link>
          </div>
        </form>
      </div>
    </div>
  );
}
