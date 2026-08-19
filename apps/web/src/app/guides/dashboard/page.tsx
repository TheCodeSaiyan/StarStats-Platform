import type { Metadata, Route } from 'next';
import Link from 'next/link';

export const metadata: Metadata = {
  title: 'Your dashboard',
  description:
    'The tiles on /me, the range and lens controls that drive all of them, and why All means 365 days rather than all time.',
};

/* Facts traced at v1.8.167, not remembered:
 *   - ranges + default: lib/range.ts RANGES / DEFAULT_RANGE ('7d')
 *   - "All" = 365 days: rangeToWindowHours + its comment ("that is
 *     CORRECT, not a bug: 365 days is the hard retention limit")
 *   - lenses: lib/lens.ts LENSES
 *   - rail items: components/shell/LeftRail.tsx
 *
 * Widget descriptions are deliberately NOT reproduced here.
 * widgets/widget-meta.ts is the single source of truth and the palette
 * renders from it; a copy on this page would drift the first time a
 * widget changed. Point at the palette, don't transcribe it.
 *
 * Range leads because it is the one number on the site that means
 * something other than what it says. A user comparing totals against a
 * mate's will otherwise conclude data is missing. */
export default function DashboardGuidePage() {
  return (
    <main className="ss-about">
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
        Your dashboard.
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
        Two controls change almost everything on screen, and one of them
        doesn&apos;t mean what it says. Those go first.
      </p>

      <section className="ss-about-section" id="me">
        <div className="ss-about-section-eyebrow">01 — Where you are</div>
        <h2>Everything starts at /me.</h2>
        <p>
          <Link href={'/me' as Route}>/me</Link> is a grid of tiles, each
          showing one thing about how you play. It&apos;s the only page
          that is entirely yours — nobody sees it unless you{' '}
          <Link href={'/guides/sharing' as Route}>share it</Link>.
        </p>
        <p style={{ color: 'var(--fg-muted)' }}>
          The left rail has the three places you&apos;ll actually go:{' '}
          <strong>Me</strong>, <strong>Discover</strong> (other players who
          made their profile public) and <strong>Orgs</strong>.
        </p>
      </section>

      <section className="ss-about-section" id="range">
        <div className="ss-about-section-eyebrow">02 — Range</div>
        <h2>&ldquo;All&rdquo; means 365 days, not all time.</h2>
        <p>
          Every tile obeys one range control: <code>24h</code>,{' '}
          <code>7d</code>, <code>30d</code>, <code>90d</code> or{' '}
          <code>All</code>. A fresh visit defaults to <strong>7d</strong>.
        </p>
        <p>
          <code>All</code> is 365 days because 365 days is the retention
          limit — we don&apos;t keep events older than that, so
          &ldquo;everything we have&rdquo; and &ldquo;a year&rdquo; are the
          same window. Calling it &ldquo;all time&rdquo; would be a lie by
          one word, and you&apos;d find out by wondering where a
          year-old flight went.
        </p>
        <p style={{ color: 'var(--fg-muted)' }}>
          Numbers come with a comparison wherever an honest one exists — a
          figure on its own tells you nothing about whether it&apos;s a
          lot. Where a tile can compare this window against the one before
          it, it does, and says so.
        </p>
      </section>

      <section className="ss-about-section" id="lens">
        <div className="ss-about-section-eyebrow">03 — Lens</div>
        <h2>Hide the tiles you&apos;re not asking about.</h2>
        <p>
          The lens filters the grid to one subject:{' '}
          <strong>All</strong>, <strong>Activity</strong>,{' '}
          <strong>Travel</strong>, <strong>Combat</strong>,{' '}
          <strong>Loadout</strong> or <strong>Commerce</strong>.
        </p>
        <p>
          It hides tiles rather than changing them. A tile shows the same
          thing under every lens, so switching lenses can never make a
          number disagree with itself.
        </p>
      </section>

      <section className="ss-about-section" id="tiles">
        <div className="ss-about-section-eyebrow">04 — Arranging tiles</div>
        <h2>Drag, resize, add, remove. It saves itself.</h2>
        <p>
          Tiles drag to reorder and resize from the corner. Each has a
          floor and a ceiling: it can&apos;t shrink below the size where
          its main number still fits, and it can&apos;t grow big enough to
          swallow the dashboard. The layout is stored on your account, so
          it follows you to another browser.
        </p>
        <p>
          <strong>Add widget</strong> opens the palette — every available
          tile with a line describing what it shows. The palette is the
          live list, which is why it isn&apos;t reproduced here: a copy on
          this page would be wrong the first time a tile changed.
        </p>
        <p style={{ color: 'var(--fg-muted)' }}>
          Tiles never scroll inside themselves. One with more to say than
          fits shows a bounded summary and a way through to the full page —
          which is why some have a &ldquo;see more&rdquo; and others
          don&apos;t.
        </p>
      </section>

      <section className="ss-about-section" id="deeper">
        <div className="ss-about-section-eyebrow">05 — The deeper pages</div>
        <h2>Three subjects outgrew their tiles.</h2>
        <p>
          <Link href={'/me/travel' as Route}>/me/travel</Link> — where
          you&apos;ve been, the routes you fly most, and the legs between
          them.
        </p>
        <p>
          <Link href={'/me/loadout' as Route}>/me/loadout</Link> — your
          gear, as a paperdoll.
        </p>
        <p>
          <Link href={'/me/contracts' as Route}>/me/contracts</Link> —
          every contract run, with outcomes and a history.
        </p>
        <p style={{ color: 'var(--fg-muted)' }}>
          Ship, item and place names across the site are links. Following
          one lands you in the{' '}
          <Link href={'/kb' as Route}>knowledge base</Link> entry for that
          thing.
        </p>
      </section>

      <section className="ss-about-section">
        <div className="ss-about-section-eyebrow">Empty tiles?</div>
        <h2>Usually the app, not the page.</h2>
        <p>
          A tile with nothing in it almost always means the desktop app
          isn&apos;t sending — not that the site is broken. Check{' '}
          <strong>Tailing</strong> and <strong>Remote sync</strong> on the
          app&apos;s Readout tab first (
          <Link href={'/guides/desktop-app' as Route}>where those are</Link>
          ), then{' '}
          <Link href={'/docs/troubleshooting' as Route}>troubleshooting</Link>.
        </p>
        <p style={{ color: 'var(--fg-muted)' }}>
          One exception worth knowing: anything phrased in clock time stays
          empty until you set a timezone in{' '}
          <Link href={'/guides/settings' as Route}>settings</Link>. That
          one isn&apos;t a fault, it&apos;s a missing answer.
        </p>
      </section>
    </main>
  );
}
