import React from 'react';
import type { Route } from 'next';
import { redirect } from 'next/navigation';
import { ApiCallError, getMe, signup } from '@/lib/api';
import { logger } from '@/lib/logger';
import { authAttemptsTotal } from '@/lib/metrics';
import { setSession } from '@/lib/session';
import { isBetaGateOn } from '@/lib/beta-gate';
import { BetaBanner } from '@/app/_components/BetaBanner';
import { WaitlistForm } from '@/app/_components/WaitlistForm';
import Link from 'next/link';

export const metadata = { title: "Sign up" };

interface SearchParams {
  error?: string;
  /** Prefills the invite field so an emailed link works in one click.
   *  The waitlist admission email carries `?invite=<token>`. */
  invite?: string;
}

export default async function SignupPage(props: {
  searchParams: Promise<SearchParams>;
}) {
  const { error, invite } = await props.searchParams;
  const inviteFromQuery = invite?.trim() ?? '';
  const gateOn = await isBetaGateOn();

  async function action(formData: FormData) {
    'use server';

    const email = String(formData.get('email') ?? '').trim();
    const password = String(formData.get('password') ?? '');
    const claimedHandle = String(formData.get('claimed_handle') ?? '').trim();
    const inviteToken = String(formData.get('invite_token') ?? '').trim();

    try {
      const auth = await signup({
        email,
        password,
        claimed_handle: claimedHandle,
        // Always sent when present. The server ignores it while the gate
        // is off and REQUIRES it while the gate is on, so gating this
        // client-side would reintroduce the bug it fixes: until this
        // field existed, turning the gate on made signup impossible for
        // everyone, invite holders included.
        ...(inviteToken ? { invite_token: inviteToken } : {}),
      });
      // Brand-new accounts are nearly always unverified, but fetch
      // /v1/auth/me to be honest about the source of truth. Failure
      // here is non-fatal — degrade to `false` so the verify banner
      // shows up.
      let emailVerified = false;
      let staffRoles: string[] = [];
      try {
        const me = await getMe(auth.token);
        emailVerified = me.email_verified;
        staffRoles = me.staff_roles ?? [];
      } catch (meErr) {
        logger.warn({ err: meErr }, 'getMe after signup failed; defaulting emailVerified=false');
      }
      await setSession({
        token: auth.token,
        userId: auth.user_id,
        claimedHandle: auth.claimed_handle,
        emailVerified,
        staffRoles,
      });
      authAttemptsTotal.inc({ action: 'signup', outcome: 'success' });
      logger.info({ user_id: auth.user_id }, 'signup success');
    } catch (e) {
      if (e instanceof ApiCallError) {
        authAttemptsTotal.inc({ action: 'signup', outcome: e.body.error });
        logger.info({ reason: e.body.error }, 'signup rejected');
        const params = new URLSearchParams({ error: e.body.error });
        if (inviteToken) params.set('invite', inviteToken);
        redirect(`/auth/signup?${params.toString()}`);
      }
      authAttemptsTotal.inc({ action: 'signup', outcome: 'unexpected' });
      logger.error({ err: e }, 'signup failed unexpectedly');
      const params = new URLSearchParams({ error: 'unexpected' });
      if (inviteToken) params.set('invite', inviteToken);
      redirect(`/auth/signup?${params.toString()}`);
    }
    redirect('/downloads');
  }

  if (gateOn && !inviteFromQuery) {
    return (
      <div className="hp-authpage">
        <div className="hp-authcard">
          <span className="ss-eyebrow">Private beta</span>
          <h1>Join the StarStats beta.</h1>
          <p className="hp-authsub">
            Account creation is invite-only for now. Join the waitlist and
            we&rsquo;ll email your signup link when a place opens.
          </p>

          <WaitlistForm source="auth-signup" />

          <hr className="ss-rule" />

          <p style={{ margin: 0, fontSize: 13, color: 'var(--fg-muted)' }}>
            Already have an account?{' '}
            <Link href="/auth/login" style={{ color: 'var(--accent)' }}>
              Sign in
            </Link>
            . If you&rsquo;ve received an invite, use the signup link in that
            email.
          </p>
        </div>
      </div>
    );
  }

  return (
    <div className="hp-authpage">
      <div className="hp-authcard">
        {gateOn && <BetaBanner mode="signup" />}
        <span className="ss-eyebrow">Create account</span>
        <h1>Create your account.</h1>
        <p className="hp-authsub">
          Email plus a password gets you an account. You can verify your RSI
          handle later.
        </p>

        {error && (
          <div className="ss-alert ss-alert--danger" role="alert">
            {labelForError(error)}
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

          <label className="ss-label">
            <span className="ss-label-text">Password</span>
            <input
              className="ss-input"
              type="password"
              name="password"
              required
              minLength={12}
              autoComplete="new-password"
            />
            <small style={{ color: 'var(--fg-dim)', fontSize: 12 }}>
              At least 12 characters.
            </small>
          </label>

          <label className="ss-label">
            <span className="ss-label-text">RSI handle</span>
            <input
              className="ss-input"
              type="text"
              name="claimed_handle"
              required
              autoComplete="username"
              placeholder="TheCodeSaiyan"
              spellCheck={false}
            />
            <small style={{ color: 'var(--fg-dim)', fontSize: 12 }}>
              The handle that appears in your Game.log.
            </small>
          </label>

          {/* Rendered only while the gate is on — an invite field on an
              open signup is a barrier that does not exist. `required`
              likewise tracks the gate: the server rejects a missing
              token with `invite_required`, so failing here with a
              browser hint beats a round trip. Prefilled from `?invite=`
              so the waitlist email is one click. */}
          {gateOn && (
            <label className="ss-label">
              <span className="ss-label-text">Invite code</span>
              <input
                className="ss-input"
                type="text"
                name="invite_token"
                required
                defaultValue={inviteFromQuery}
                autoComplete="off"
                spellCheck={false}
                placeholder="From your waitlist email"
              />
              <small style={{ color: 'var(--fg-dim)', fontSize: 12 }}>
                StarStats is invite-only during the beta.
              </small>
            </label>
          )}

          <button
            type="submit"
            className="ss-btn ss-btn--primary"
            style={{ marginTop: 6 }}
          >
            Create account
          </button>
        </form>

        <p className="hp-authfine">
          By creating an account you agree to our{' '}
          <Link href={'/terms' as Route} style={{ color: 'var(--fg-muted)' }}>
            Terms of Service
          </Link>{' '}
          and acknowledge our{' '}
          <Link href="/privacy" style={{ color: 'var(--fg-muted)' }}>
            Privacy Policy
          </Link>
          . We process your email for authentication and account recovery
          (contract performance, GDPR Art. 6(1)(b)) and your RSI handle to tag
          the game events you choose to upload.
        </p>

        <hr className="ss-rule" />

        <p
          style={{
            margin: 0,
            fontSize: 13,
            color: 'var(--fg-muted)',
          }}
        >
          Already have an account?{' '}
          <Link href="/auth/login" style={{ color: 'var(--accent)' }}>
            Sign in
          </Link>
          .
        </p>
      </div>
    </div>
  );
}

function labelForError(code: string): string {
  switch (code) {
    case 'invalid_email':
      return "That email doesn't look right — make sure it has @ and a domain.";
    case 'password_too_short':
      return 'Password must be at least 12 characters.';
    case 'missing_handle':
      return 'RSI handle is required.';
    case 'email_taken':
      return 'An account with that email already exists.';
    case 'handle_taken':
      return 'Someone else already claimed that RSI handle.';
    // The server distinguishes these two, and so must we: telling
    // someone with a spent invite to "join the waitlist" sends them
    // back to a queue they have already come through.
    case 'invite_required':
      return 'StarStats is in invite-only beta — enter the invite code from your waitlist email.';
    case 'invite_invalid':
      return 'That invite code is not valid, or has already been used. Check the code, or contact us if you think it should still work.';
    case 'gate_unavailable':
      return "We couldn't verify the beta invite list just now. Please try again in a moment.";
    default:
      return "Something went wrong. Please try again, or check the URL bar's error code.";
  }
}
