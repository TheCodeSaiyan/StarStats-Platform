import { MarketingSurface } from '@/components/projection/MarketingSurface';
import { DocsIndex } from '@/components/projection/DocsIndex';
import type { Metadata, Route } from 'next';
import Link from 'next/link';

export const metadata: Metadata = {
  title: 'Guides',
  description:
    'How to use StarStats once it is running — the desktop app, your dashboard, sharing, and the settings that change what you see.',
};

/* Guides vs Docs, and why they are separate top-level sections:
 *
 *   /docs   — getting it working. Install, pair, cookie, verify, sync,
 *             plus the two reference pages (rsi-cookie, troubleshooting).
 *   /guides — using it. Assumes it already works.
 *
 * A visitor with nothing installed and a visitor whose tiles are empty
 * want different pages, and one combined section serves neither. The nav
 * label matches the URL so the distinction survives being linked to.
 *
 * Facts on the child pages are traced, not remembered — /docs learned
 * that the hard way (its header records three premises that were wrong
 * when checked). Each guide names the source it was read from. */
export default function GuidesIndexPage() {
  return (
    <MarketingSurface
      navId="guides"
      crumb={[
        { label: 'Site', href: '/' },
        { label: 'Guides' },
      ]}
      title="Guides"
      ctx="Walkthroughs for each surface"
    >
      <DocsIndex active="/guides" />
    <div className="ss-about">
      <div className="ss-placard" style={{ marginBottom: 'var(--s5)' }}>
        Guides
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
        Using StarStats.
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
        These pages assume it&apos;s already working. If it isn&apos;t
        yet, <Link href={'/docs' as Route}>the setup steps</Link> come
        first — there are five and four of them you wouldn&apos;t guess.
      </p>

      <section className="ss-about-section" id="app">
        <div className="ss-about-section-eyebrow">The desktop app</div>
        <h2>
          <Link href={'/guides/desktop-app' as Route}>
            What the six tabs do
          </Link>
        </h2>
        <p>
          Four of the six aren&apos;t named after their job —{' '}
          <strong>Calibrate</strong> is Settings, and that&apos;s the one
          everybody hunts for. Also covers the Review queue, the settings
          worth knowing, and why <strong>re-parse</strong> and{' '}
          <strong>re-ingest</strong> are different buttons.
        </p>
      </section>

      <section className="ss-about-section" id="site">
        <div className="ss-about-section-eyebrow">The website</div>
        <h2>Three guides, because it&apos;s three jobs.</h2>
        <p>
          <Link href={'/guides/dashboard' as Route}>Your dashboard</Link> —
          the tiles, and the two controls that change all of them at once.
          Including why <code>All</code> means 365 days rather than all
          time.
        </p>
        <p>
          <Link href={'/guides/sharing' as Route}>Sharing</Link> — private
          by default, and two separate levels: who can see you, and which
          categories they see.
        </p>
        <p>
          <Link href={'/guides/settings' as Route}>Settings</Link> — the
          account pages, and the one setting that leaves tiles empty until
          you save it.
        </p>
      </section>

      <section className="ss-about-section">
        <div className="ss-about-section-eyebrow">Not working?</div>
        <h2>That&apos;s a different page.</h2>
        <p>
          <Link href={'/docs/troubleshooting' as Route}>Troubleshooting</Link>{' '}
          covers the six things that account for most of it — two of which
          aren&apos;t bugs and never will be.
        </p>
      </section>
    </div>
    </MarketingSurface>
  );
}
