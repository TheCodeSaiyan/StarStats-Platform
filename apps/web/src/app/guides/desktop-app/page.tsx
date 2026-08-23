import { MarketingSurface } from '@/components/projection/MarketingSurface';
import { DocsIndex } from '@/components/projection/DocsIndex';
import type { Metadata, Route } from 'next';
import Link from 'next/link';

export const metadata: Metadata = {
  title: 'Using the desktop app',
  description:
    'What each tab in the StarStats desktop app does, which settings matter, and the difference between re-parse and re-ingest.',
};

/* The usage half of /docs. /docs gets you connected; this page is what to
 * do with the thing once it is running.
 *
 * Facts here are traced, not remembered — same standard as /docs, which
 * had three wrong premises when they were checked. Every tab name, card
 * title and default below was read out of the source at v1.8.62:
 *   - tab labels: TrayHeader.tsx TAB_LABELS (branded) + TAB_TITLES (plain)
 *   - card titles: StatusPane.tsx / SettingsPane.tsx `TrayCard title=`
 *   - re-ingest behaviour: ReingestCard.tsx header comment
 *
 * The tab-name table leads because it is the one thing a new user cannot
 * work out: TrayHeader's own comment concedes "a new user can't guess
 * 'Calibrate' = Settings". Every other doc that says "open Settings" is
 * pointing at a tab that is not labelled Settings. */
export default function DesktopAppGuidePage() {
  return (
    <MarketingSurface
      navId="guides"
      crumb={[
        { label: 'Site', href: '/' },
        { label: 'Guides', href: '/guides' },
        { label: 'Desktop app' },
      ]}
      title="Desktop app"
      ctx="Guides · the emitter, pane by pane"
    >
      <DocsIndex active="/guides/desktop-app" />
    <div className="ss-about">
      <div className="ss-placard" style={{ marginBottom: 'var(--s5)' }}>
        Docs
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
        Using the desktop app.
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
        Six tabs, and four of them aren&apos;t called what they do. That
        table comes first, because every other instruction on this page
        depends on you finding the right tab.
      </p>

      <section className="ss-about-section" id="tabs">
        <div className="ss-about-section-eyebrow">01 — The tabs</div>
        <h2>Settings is called Calibrate.</h2>
        <p>
          The tab labels are deliberately in the app&apos;s own voice.
          That reads well and hides what they do, so here is the map. Hover
          any tab in the app and it tells you the same thing.
        </p>
        <dl
          style={{
            margin: '0 0 var(--s4)',
            display: 'grid',
            /* Sizes to the longest term ("What's New", ~5rem) rather than
             * a fixed column, so the description keeps the most room at
             * 375px. Inline styles can't carry a media query and this
             * page doesn't warrant a shared class. */
            gridTemplateColumns: 'minmax(0, max-content) 1fr',
            gap: 'var(--s2) var(--s5)',
            lineHeight: 1.6,
          }}
        >
          <dt style={{ fontWeight: 600 }}>Readout</dt>
          <dd style={{ margin: 0, color: 'var(--fg-muted)' }}>
            Status. The tab you actually look at.
          </dd>
          <dt style={{ fontWeight: 600 }}>Manifest</dt>
          <dd style={{ margin: 0, color: 'var(--fg-muted)' }}>
            Logs — the raw lines, as read.
          </dd>
          <dt style={{ fontWeight: 600 }}>Catalogue</dt>
          <dd style={{ margin: 0, color: 'var(--fg-muted)' }}>
            Knowledge base — ships, items, places.
          </dd>
          <dt style={{ fontWeight: 600 }}>What&apos;s New</dt>
          <dd style={{ margin: 0, color: 'var(--fg-muted)' }}>
            Release notes for the version you&apos;re running.
          </dd>
          <dt style={{ fontWeight: 600 }}>Review</dt>
          <dd style={{ margin: 0, color: 'var(--fg-muted)' }}>
            Lines StarStats couldn&apos;t read. Carries a badge when there
            are any.
          </dd>
          <dt style={{ fontWeight: 600 }}>Calibrate</dt>
          <dd style={{ margin: 0, color: 'var(--fg-muted)' }}>
            <strong>Settings.</strong> This is the one people hunt for.
          </dd>
        </dl>
      </section>

      <section className="ss-about-section" id="readout">
        <div className="ss-about-section-eyebrow">02 — Readout</div>
        <h2>One line tells you whether anything is working.</h2>
        <p>
          <strong>Tailing</strong> is the card that matters. If it names a
          file, the app is watching your log right now and everything else
          on this page is a detail. If it doesn&apos;t, nothing is being
          recorded and no amount of re-parsing will help —{' '}
          <Link href={'/docs/troubleshooting' as Route}>troubleshooting</Link>{' '}
          starts there for that reason.
        </p>
        <p>
          Below it: <strong>Recent activity</strong> over the last 48 hours,{' '}
          <strong>Top event types</strong>, a <strong>Session timeline</strong>,
          and <strong>Parser coverage</strong> — how much of your log the app
          understood, which is the honest version of &ldquo;is this
          working&rdquo;. <strong>Sources</strong> and{' '}
          <strong>Discovered logs</strong> show which files it found.
        </p>
        <p style={{ color: 'var(--fg-muted)' }}>
          <strong>Remote sync</strong> reports whether this machine is
          paired and sending. Paired and tailing are different things: a
          machine can be reading your log perfectly and sending none of it.
        </p>
      </section>

      <section className="ss-about-section" id="review">
        <div className="ss-about-section-eyebrow">03 — Review</div>
        <h2>The badge is a queue, and clearing it makes the parser better.</h2>
        <p>
          Star Citizen writes lines StarStats doesn&apos;t recognise —
          new patches invent them constantly. Those land in{' '}
          <strong>Review</strong> with a count on the tab. For each one you
          can submit it, attribute it to your handle, or dismiss it.
        </p>
        <p>
          Submitting sends the shape of the line so it can become a
          supported event type. Dismissing drops it locally and stops the
          app asking again. Neither is urgent — an empty queue is not a
          goal, it&apos;s just tidier.
        </p>
        <p style={{ color: 'var(--fg-muted)' }}>
          This is the only place in the app where you decide what leaves
          your machine line by line, which is why it&apos;s a queue and not
          a background job.
        </p>
      </section>

      <section className="ss-about-section" id="calibrate">
        <div className="ss-about-section-eyebrow">04 — Calibrate</div>
        <h2>The settings worth knowing about.</h2>
        <p>
          <strong>Game.log</strong> — where your log lives. Set
          automatically; change it only if you moved your install or run
          more than one.
        </p>
        <p>
          <strong>Cloud sync</strong> — off by default, greyed out until the
          app is paired, and it holds your changes until you press{' '}
          <strong>Save</strong>. Covered as step 05 on{' '}
          <Link href={'/docs' as Route}>the setup page</Link>.
        </p>
        <p>
          <strong>Updates</strong> — whether to check on launch, and which
          channel to follow. You can switch channels whenever you like; the
          next check reads the new channel&apos;s manifest.
        </p>
        <p>
          <strong>RSI session cookie</strong> — optional, and worth reading{' '}
          <Link href={'/docs/rsi-cookie' as Route}>its own page</Link>{' '}
          before you paste anything. Without it, everything works except
          hangar data.
        </p>
        <p>
          <strong>Unreadable log entries</strong> — off by default. Sends
          the names of log entry types the app couldn&apos;t read, never the
          lines themselves. It&apos;s how we spot a game update breaking
          tracking before you have to report it.
        </p>
        <p style={{ color: 'var(--fg-muted)' }}>
          <strong>Appearance</strong> and{' '}
          <strong>Org platform connector</strong> live here too. Appearance
          follows you to other machines if cloud sync is on.
        </p>
      </section>

      <section className="ss-about-section" id="reparse">
        <div className="ss-about-section-eyebrow">05 — Re-parse vs re-ingest</div>
        <h2>Two buttons, and picking the wrong one wastes an hour.</h2>
        <p>
          <strong>Re-parse local store</strong> runs the current classifier
          over lines the app has already stored. It&apos;s fast. It can only
          re-read what was kept.
        </p>
        <p>
          <strong>Re-ingest rotated logs</strong> walks every rotated{' '}
          <code>Game-*.log</code> still on disk and feeds each line through
          the classifier again, from source. It takes minutes.
        </p>
        <p>
          The distinction that matters: if an event type was{' '}
          <em>never recognised in the first place</em>, re-parsing
          can&apos;t recover it — those lines were never stored as events.
          Re-ingest goes back to the original files and can. So after an
          update that adds support for something, re-ingest is the one that
          fills the gap.
        </p>
        <p style={{ color: 'var(--fg-muted)' }}>
          Rotated logs only survive as long as your disk keeps them. If the
          files are gone, so is the history — no button brings back a log
          Star Citizen has already overwritten.
        </p>
      </section>

      <section className="ss-about-section">
        <div className="ss-about-section-eyebrow">Next</div>
        <h2>The website is the other half.</h2>
        <p>
          The desktop app reads and sends. Everything you actually{' '}
          <em>look at</em> lives on the site —{' '}
          <Link href={'/guides/dashboard' as Route}>your dashboard</Link>{' '}
          covers the tiles and the two controls that drive them, and{' '}
          <Link href={'/guides/sharing' as Route}>sharing</Link> covers who
          else gets to see them.
        </p>
      </section>
    </div>
    </MarketingSurface>
  );
}
