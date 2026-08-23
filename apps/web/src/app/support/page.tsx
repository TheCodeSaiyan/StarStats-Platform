import { MarketingSurface } from '@/components/projection/MarketingSurface';
import { DocsIndex } from '@/components/projection/DocsIndex';
import type { Metadata, Route } from 'next';
import Link from 'next/link';

export const metadata: Metadata = {
  title: 'Support',
  description:
    'Get StarStats working, report a bug, ask a question, or support the project.',
};

const REPO = 'https://github.com/TheCodeSaiyan/StarStats-Platform';
const DISCUSSIONS = `${REPO}/discussions`;
const NEW_ISSUE = `${REPO}/issues/new/choose`;
const CONTACT_EMAIL = 'dojo@thecodesaiyan.io';

/* Anonymous help page. Was a donation checkout (auth-gated) — moved to
 * /donate. During a bug-hunting beta, "Support" must mean help, and a
 * help-seeker must never be bounced to a login or a payment page. No
 * getSession here, on purpose. Destinations are verified real: /docs
 * exists, the issue chooser + Discussions are GitHub, the email is the
 * same CONTACT_EMAIL used on /trust and the waitlist form. There is no
 * Discord — do not add one. */
export default function SupportPage() {
  return (
    <MarketingSurface
      crumb={[
        { label: 'Site', href: '/' },
        { label: 'Support' },
      ]}
      title="Support"
      ctx="Getting help"
    >
      <DocsIndex active="/support" />
    <div className="ss-about">
      <div className="ss-placard" style={{ marginBottom: 'var(--s5)' }}>
        Support
      </div>

      <h1
        style={{
          fontSize: 'clamp(40px, 6vw, 64px)',
          letterSpacing: 'var(--tracking-tight)',
          lineHeight: 1.05,
          margin: '0 0 var(--s4)',
          fontWeight: 600,
        }}
      >
        Stuck? Start here.
      </h1>

      <p
        className="ss-lede"
        style={{
          fontSize: 'var(--fs-lg)',
          color: 'var(--fg-muted)',
          lineHeight: 1.55,
          margin: '0 0 var(--s7)',
          maxWidth: '60ch',
        }}
      >
        Four ways to get unstuck, in the order most people need them.
      </p>

      <section className="ss-about-section">
        <div className="ss-about-section-eyebrow">01 — Get it working</div>
        <h2>The setup guide covers most of it.</h2>
        <p>
          Install, pairing, the RSI cookie, verifying your handle, and the
          usual snags are all written up in{' '}
          <Link href={'/docs' as Route}>the docs</Link>.
        </p>
      </section>

      <section className="ss-about-section">
        <div className="ss-about-section-eyebrow">02 — Something&apos;s broken</div>
        <h2>File it — that&apos;s what the beta is for.</h2>
        <p>
          A parser gap or a bug only gets fixed when someone reports it.
          Open one from the{' '}
          <a href={NEW_ISSUE}>issue templates</a> (bug report or
          parser-rule request) so it lands with the context we need.
        </p>
      </section>

      <section className="ss-about-section">
        <div className="ss-about-section-eyebrow">03 — Talk to us</div>
        <h2>Questions, ideas, or not sure it&apos;s a bug.</h2>
        <p>
          Open-ended things go in{' '}
          <a href={DISCUSSIONS}>GitHub Discussions</a>. Or email{' '}
          <a href={`mailto:${CONTACT_EMAIL}`}>{CONTACT_EMAIL}</a> — it
          reaches the maintainer directly.
        </p>
      </section>

      <section className="ss-about-section">
        <div className="ss-about-section-eyebrow">04 — Support the project</div>
        <h2>If you want to chip in.</h2>
        <p>
          StarStats is free. If you&apos;d like to help cover the running
          costs, there&apos;s a{' '}
          <Link href={'/donate' as Route}>donation page</Link> — entirely
          optional, and never required for anything.
        </p>
      </section>
    </div>
    </MarketingSurface>
  );
}
