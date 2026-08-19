/**
 * Donate landing — Revolut Business hosted-checkout entry point.
 *
 * Flow:
 *   1. Auth gate — anonymous users get bounced to /auth/login.
 *   2. Server Component fetches `/v1/donate/tiers` + the caller's
 *      supporter status. The tier list is static today (server const)
 *      but routed through the API so a future tier edit doesn't need
 *      a frontend rebuild.
 *   3. Each tier renders a `<form action={createCheckout}>` Server
 *      Action. On submit we hit `/v1/donate/checkout`, get back a
 *      hosted-checkout URL, and `redirect()` the browser to it.
 *   4. Revolut redirects back to `/donate/return` after the customer
 *      pays — that page polls `/v1/me/supporter` until the webhook
 *      lands and the status flips to `active`.
 *
 * The whole flow short-circuits to a "coming soon" panel when the
 * server returns 503 `not_configured` (no REVOLUT_API_KEY in env).
 * Same posture as the magic-link / SpiceDB routes when their backing
 * service is offline.
 */

import Link from 'next/link';
import { redirect } from 'next/navigation';
import type { Route } from 'next';
import {
  ApiCallError,
  getSupporterStatus,
  listDonateTiers,
  startDonateCheckout,
  type SupporterStatusDto,
  type TierDto,
} from '@/lib/api';
import { getSession } from '@/lib/session';

export const metadata = { title: "Donate" };

// Moved back to /donate (verbatim) — `/support` is being rebuilt as an
// anonymous help page in a follow-up task. The Server Action and
// lib/api endpoints still hit `/v1/donate/...` — that's the backend
// product path, deliberately kept stable across the URL move.
export default async function DonatePage() {
  const session = await getSession();
  if (!session) redirect('/auth/login?next=/donate');

  let supporter: SupporterStatusDto;
  try {
    supporter = await getSupporterStatus(session.token);
  } catch (e) {
    if (e instanceof ApiCallError && e.status === 401) {
      redirect('/auth/login?next=/donate');
    }
    throw e;
  }

  // Tier list. We attempt to fetch it; if the server has Revolut
  // disabled the tiers route still works (it's a static list), so a
  // failure here means something else is wrong and we should show the
  // "coming soon" path.
  let tiers: TierDto[] = [];
  let tierFetchOk = true;
  try {
    const resp = await listDonateTiers();
    tiers = resp.tiers;
  } catch {
    tierFetchOk = false;
  }

  const isSupporter =
    supporter.state === 'active' || supporter.state === 'lapsed';

  return (
    <div
      className="ss-screen-enter"
      style={{ display: 'flex', flexDirection: 'column', gap: 20, maxWidth: 720 }}
    >
      <header>
        <div className="ss-eyebrow" style={{ marginBottom: 8 }}>
          Supporter · keep StarStats flying
        </div>
        <h1
          style={{
            margin: 0,
            fontSize: 32,
            fontWeight: 600,
            letterSpacing: '-0.02em',
          }}
        >
          Donate
        </h1>
        <p style={{ margin: '6px 0 0', color: 'var(--fg-muted)', fontSize: 14 }}>
          StarStats is free to use. If you find it useful, you can chip in to
          cover hosting and keep the lights on. Donations go through Revolut
          Business; payment processing is handled by Revolut, not by us — we
          never see your card details.
        </p>
      </header>

      {isSupporter && <CurrentStatus status={supporter} />}

      {tierFetchOk && tiers.length > 0 ? (
        <TierGrid tiers={tiers} existingPlate={supporter.name_plate ?? null} />
      ) : (
        <ComingSoonPanel />
      )}

      <section
        className="ss-card"
        style={{ padding: '18px 20px', background: 'var(--bg-elev)' }}
      >
        <div className="ss-eyebrow" style={{ marginBottom: 8 }}>
          What supporters get
        </div>
        <ul
          style={{
            margin: 0,
            paddingLeft: 18,
            color: 'var(--fg-muted)',
            fontSize: 13,
            lineHeight: 1.7,
          }}
        >
          <li>A supporter pill on your public profile.</li>
          <li>An optional 28-char name plate alongside it.</li>
          <li>Extended retention on your stored events.</li>
          <li>An accent-tinted UI affordance on the dashboard.</li>
          <li>
            A warm fuzzy feeling. The first three are durable — even if you
            stop donating later, the pill and name plate stay.
          </li>
        </ul>
      </section>

      <p style={{ margin: 0, fontSize: 12, color: 'var(--fg-dim)' }}>
        Want to help in a different way?{' '}
        <Link
          href="/submissions"
          style={{ color: 'var(--accent)', textDecoration: 'underline' }}
        >
          Submit a parser pattern
        </Link>{' '}
        — community-curated rules ship in the next parser update.
      </p>
    </div>
  );
}

function CurrentStatus({ status }: { status: SupporterStatusDto }) {
  const stateLabel = status.state === 'active' ? 'Active' : 'Lapsed';
  const stateColour =
    status.state === 'active' ? 'var(--ok)' : 'var(--fg-muted)';
  return (
    <section
      className="ss-card"
      style={{
        padding: '16px 20px',
        borderColor:
          status.state === 'active' ? 'var(--accent)' : 'var(--border)',
      }}
    >
      <div className="ss-eyebrow" style={{ marginBottom: 8 }}>
        Your supporter status
      </div>
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 12,
          flexWrap: 'wrap',
        }}
      >
        <span
          className="mono"
          style={{
            fontSize: 16,
            fontWeight: 500,
            color: stateColour,
          }}
        >
          {stateLabel}
        </span>
        {status.name_plate && (
          <span
            className="mono"
            style={{
              fontSize: 13,
              color: 'var(--accent)',
              padding: '4px 10px',
              borderRadius: 'var(--r-pill)',
              background: 'color-mix(in oklab, var(--accent) 12%, transparent)',
              border: '1px solid var(--accent)',
            }}
          >
            {status.name_plate}
          </span>
        )}
      </div>
      {status.state === 'lapsed' && (
        <p
          style={{
            margin: '10px 0 0',
            color: 'var(--fg-dim)',
            fontSize: 12,
            lineHeight: 1.6,
          }}
        >
          Your supporter pill stays — recognition is permanent. Retention
          extension and accent perks revert to free-tier until the next
          payment lands.
        </p>
      )}
    </section>
  );
}

function ComingSoonPanel() {
  return (
    <section className="ss-card" style={{ padding: '20px 22px' }}>
      <div className="ss-eyebrow" style={{ marginBottom: 8 }}>
        Status
      </div>
      <h2
        style={{
          margin: 0,
          fontSize: 20,
          fontWeight: 500,
          letterSpacing: '-0.01em',
        }}
      >
        Coming soon
      </h2>
      <p
        style={{
          margin: '8px 0 0',
          color: 'var(--fg-muted)',
          fontSize: 14,
          lineHeight: 1.55,
        }}
      >
        The Revolut Business checkout integration is wired up but waiting on
        production credentials. Once the server has{' '}
        <code className="mono">REVOLUT_API_KEY</code> set, this page lights up
        with tier buttons. Full plan in{' '}
        <code className="mono">docs/REVOLUT-INTEGRATION-PLAN.md</code>.
      </p>
    </section>
  );
}

function TierGrid({
  tiers,
  existingPlate,
}: {
  tiers: TierDto[];
  existingPlate: string | null;
}) {
  return (
    <section className="ss-card" style={{ padding: '20px 22px' }}>
      <div className="ss-eyebrow" style={{ marginBottom: 8 }}>
        Pick a tier
      </div>
      <p
        style={{
          margin: '0 0 14px',
          color: 'var(--fg-muted)',
          fontSize: 13,
          lineHeight: 1.55,
        }}
      >
        Each click hands you off to Revolut&apos;s hosted checkout. We never
        see your card. After payment clears we flip your supporter status —
        usually within a few seconds of Revolut&apos;s webhook landing.
      </p>
      <div
        style={{
          display: 'grid',
          gridTemplateColumns: 'repeat(auto-fit, minmax(240px, 1fr))',
          gap: 14,
        }}
      >
        {tiers.map((tier) => (
          <TierCard
            key={tier.key}
            tier={tier}
            defaultPlate={existingPlate ?? ''}
          />
        ))}
      </div>
    </section>
  );
}

function TierCard({
  tier,
  defaultPlate,
}: {
  tier: TierDto;
  defaultPlate: string;
}) {
  const formatted = formatMinor(tier.amount_minor, tier.currency);
  return (
    <form
      action={createCheckout}
      style={{
        display: 'flex',
        flexDirection: 'column',
        gap: 10,
        padding: 16,
        borderRadius: 10,
        background: 'var(--bg-elev)',
        border: '1px solid var(--border)',
      }}
    >
      <input type="hidden" name="tier_key" value={tier.key} />
      <div
        style={{
          display: 'flex',
          alignItems: 'baseline',
          justifyContent: 'space-between',
          gap: 8,
        }}
      >
        <strong style={{ fontSize: 15, fontWeight: 600 }}>{tier.label}</strong>
        <span
          className="mono"
          style={{
            fontSize: 16,
            fontWeight: 500,
            color: 'var(--accent)',
          }}
        >
          {formatted}
        </span>
      </div>
      <p
        style={{
          margin: 0,
          fontSize: 12,
          color: 'var(--fg-muted)',
          lineHeight: 1.55,
        }}
      >
        {tier.description}
      </p>
      <label
        style={{
          display: 'flex',
          flexDirection: 'column',
          gap: 4,
          fontSize: 11,
          color: 'var(--fg-dim)',
          textTransform: 'uppercase',
          letterSpacing: '0.04em',
        }}
      >
        Name plate (optional, max 28 chars)
        <input
          type="text"
          name="name_plate"
          maxLength={28}
          defaultValue={defaultPlate}
          placeholder="e.g. Caelum"
          style={{
            padding: '6px 10px',
            background: 'var(--bg)',
            border: '1px solid var(--border)',
            borderRadius: 6,
            color: 'var(--fg)',
            fontFamily: 'var(--font-mono)',
            fontSize: 13,
            textTransform: 'none',
            letterSpacing: 'normal',
          }}
        />
      </label>
      <button
        type="submit"
        className="ss-btn ss-btn--primary"
        style={{ marginTop: 4 }}
      >
        Donate {formatted}
      </button>
    </form>
  );
}

function formatMinor(amountMinor: number, currency: string): string {
  const major = amountMinor / 100;
  try {
    return new Intl.NumberFormat('en-GB', {
      style: 'currency',
      currency,
      maximumFractionDigits: 2,
    }).format(major);
  } catch {
    return `${major.toFixed(2)} ${currency}`;
  }
}

// -- Server Action ---------------------------------------------------

async function createCheckout(formData: FormData) {
  'use server';
  const session = await getSession();
  if (!session) redirect('/auth/login?next=/donate');

  const tierKey = formData.get('tier_key');
  const namePlate = formData.get('name_plate');
  if (typeof tierKey !== 'string' || tierKey.length === 0) {
    redirect('/donate?error=invalid_tier');
  }

  const plateRaw = typeof namePlate === 'string' ? namePlate.trim() : '';
  const plate = plateRaw.length === 0 ? null : plateRaw;

  let resp;
  try {
    resp = await startDonateCheckout(session.token, {
      tier_key: tierKey,
      name_plate: plate,
    });
  } catch (e) {
    if (e instanceof ApiCallError) {
      if (e.status === 401) redirect('/auth/login?next=/donate');
      redirect(`/donate?error=${encodeURIComponent(e.body.error)}`);
    }
    throw e;
  }

  // Hand off to Revolut's hosted checkout. Server-side redirect so the
  // browser navigates rather than a JSON response landing in the form.
  // The URL is fully qualified (revolut.com or sandbox-revolut.com); we
  // cast to `Route` because Next's typedRoutes only knows about our
  // own paths and would otherwise reject any external URL here.
  redirect(resp.checkout_url as Route);
}
