/**
 * Donate return — landing page Revolut redirects to after the customer
 * completes (or abandons) checkout.
 *
 * Important caveat: Revolut redirects on success AND on cancel/back.
 * The redirect itself is NOT proof of payment — only the
 * `ORDER_COMPLETED` webhook is trusted by the server. So this page
 * just reads the caller's supporter status and shows what it currently
 * is; if the webhook hasn't landed yet (it normally arrives within a
 * few seconds) the user sees a "still processing" state and the page
 * auto-refreshes via `<meta http-equiv="refresh">`.
 *
 * No client-side polling because that would need a "use client"
 * boundary and a fresh ApiCallError; a 6-second meta-refresh is good
 * enough for the rare case where the webhook lags.
 */

import Link from 'next/link';
import { redirect } from 'next/navigation';
import {
  ApiCallError,
  getSupporterStatus,
  type SupporterStatusDto,
} from '@/lib/api';
import { getSession } from '@/lib/session';

export const metadata = { title: 'Donate' };

export default async function DonateReturnPage() {
  const session = await getSession();
  if (!session) redirect('/auth/login?next=/donate/return');

  let supporter: SupporterStatusDto;
  try {
    supporter = await getSupporterStatus(session.token);
  } catch (e) {
    if (e instanceof ApiCallError && e.status === 401) {
      redirect('/auth/login?next=/donate/return');
    }
    throw e;
  }

  const isActive = supporter.state === 'active';
  // If we just came back from checkout but the webhook hasn't landed
  // yet, the state will still be `none` (or `lapsed` for renewals).
  // Auto-refresh until it flips. Cap at this page — once active, stop
  // refreshing.
  const shouldRefresh = !isActive;

  return (
    <>
      {shouldRefresh && (
        // 6 seconds: long enough that webhook redeliveries aren't
        // racing the page refresh, short enough that a real user
        // doesn't sit there wondering.
        <meta httpEquiv="refresh" content="6" />
      )}
      <div
        className="ss-screen-enter"
        style={{ display: 'flex', flexDirection: 'column', gap: 20, maxWidth: 640 }}
      >
        <header>
          <div className="ss-eyebrow" style={{ marginBottom: 8 }}>
            Supporter · payment return
          </div>
          <h1
            style={{
              margin: 0,
              fontSize: 28,
              fontWeight: 600,
              letterSpacing: '-0.02em',
            }}
          >
            {isActive ? 'Thank you' : 'Checking with Revolut...'}
          </h1>
        </header>

        {isActive ? <ActivePanel status={supporter} /> : <PendingPanel />}

        <p style={{ margin: 0, fontSize: 12, color: 'var(--fg-dim)' }}>
          <Link
            href="/donate"
            style={{ color: 'var(--accent)', textDecoration: 'underline' }}
          >
            Back to Donate
          </Link>
          {' · '}
          <Link
            href="/me"
            style={{ color: 'var(--accent)', textDecoration: 'underline' }}
          >
            Home
          </Link>
        </p>
      </div>
    </>
  );
}

function ActivePanel({ status }: { status: SupporterStatusDto }) {
  return (
    <section
      className="ss-card"
      style={{ padding: '20px 22px', borderColor: 'var(--accent)' }}
    >
      <p
        style={{
          margin: 0,
          fontSize: 14,
          color: 'var(--fg-muted)',
          lineHeight: 1.55,
        }}
      >
        Your supporter status is{' '}
        <strong style={{ color: 'var(--ok)' }}>active</strong>. Welcome to the
        wing. Your name plate is locked in
        {status.name_plate ? (
          <>
            {' '}as{' '}
            <span
              className="mono"
              style={{
                color: 'var(--accent)',
                padding: '2px 8px',
                borderRadius: 0,
                background: 'color-mix(in oklab, var(--accent) 12%, transparent)',
                border: '1px solid var(--accent)',
              }}
            >
              {status.name_plate}
            </span>
          </>
        ) : (
          ' (you can set one anytime from settings)'
        )}
        .
      </p>
      {status.grace_until && (
        <p
          style={{
            margin: '10px 0 0',
            fontSize: 12,
            color: 'var(--fg-dim)',
            lineHeight: 1.6,
          }}
        >
          Coverage runs until{' '}
          <span className="mono">
            {new Date(status.grace_until).toLocaleDateString()}
          </span>
          . After that, the pill stays — recognition is permanent — but
          retention extension and accent perks revert to free-tier until the
          next payment lands.
        </p>
      )}
    </section>
  );
}

function PendingPanel() {
  return (
    <section className="ss-card" style={{ padding: '20px 22px' }}>
      <p
        style={{
          margin: 0,
          fontSize: 14,
          color: 'var(--fg-muted)',
          lineHeight: 1.55,
        }}
      >
        We&apos;re waiting for Revolut to confirm the payment. This usually
        takes a few seconds. The page will refresh automatically until your
        supporter status flips.
      </p>
      <p
        style={{
          margin: '10px 0 0',
          fontSize: 12,
          color: 'var(--fg-dim)',
          lineHeight: 1.6,
        }}
      >
        If the payment is taking unusually long (more than a minute), it may
        have been declined or cancelled — your card statement is the
        authoritative source. Come back to{' '}
        <code className="mono">/donate</code> any time to retry.
      </p>
    </section>
  );
}
