import type { Metadata, Route } from 'next';
import Link from 'next/link';

export const metadata: Metadata = {
  title: 'The RSI cookie',
  description:
    'What the Rsi-Token cookie is, what it grants StarStats, how long it lasts, and how to take it back.',
};

/* This is the page that decides whether a stranger trusts the project,
 * so it is blunt rather than soothing. A reassuring cookie page reads as
 * sales; naming the uncomfortable thing first and then the control is
 * what reads as honest.
 *
 * Two things NOT said here, on purpose:
 * 1. No expiry duration. The cookie lapses when RSI invalidates the
 *    session; we do not control that and must not promise a number.
 *    (`hangar.rs` has an `rsi_cookie_invalid` status precisely because
 *    they do go stale.)
 * 2. No "log out of RSI to revoke it." That is a claim about RSI's
 *    behaviour, not ours, and nothing in this repo asserts it. The
 *    revocation instruction below is the one we control and can prove:
 *    Clear cookie -> `clear_rsi_cookie` (commands.rs:2130-2134), which
 *    removes it from the OS keychain. A revocation instruction that does
 *    not revoke is the worst sentence this page could contain. */
export default function RsiCookiePage() {
  return (
    <main className="ss-about">
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
        The RSI cookie.
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
        You&apos;re being asked to paste a session cookie into a
        third-party app. That deserves a straight answer, not
        reassurance.
      </p>

      <section className="ss-about-section">
        <div className="ss-about-section-eyebrow">01 — What it is</div>
        <h2>It&apos;s your RSI login.</h2>
        <p>
          <code>Rsi-Token</code> is the cookie your browser holds after you
          sign in to RSI. It isn&apos;t a scoped API key and there is no
          read-only version of it — it is the session itself. Anything
          holding it can act as you on robertsspaceindustries.com for as
          long as it stays valid.
        </p>
        <p>
          StarStats uses it for one thing: fetching your own hangar so it
          can list what you own. But we&apos;re not going to describe it as
          &ldquo;just&rdquo; hangar access, because the cookie doesn&apos;t
          know that&apos;s all we do with it.
        </p>
      </section>

      <section className="ss-about-section">
        <div className="ss-about-section-eyebrow">02 — Where it goes</div>
        <h2>Into your keychain, not our server.</h2>
        <p>
          The desktop app stores it in your operating system&apos;s
          keychain and uses it locally. The tray says it plainly:{' '}
          <em>
            &ldquo;Never leaves your machine — only parsed ship lists are
            sent.&rdquo;
          </em>{' '}
          The cookie stays put; what reaches us is the list of ships it
          produced.
        </p>
      </section>

      <section className="ss-about-section">
        <div className="ss-about-section-eyebrow">03 — Getting it</div>
        <h2>DevTools, and nowhere else.</h2>
        <p>
          There is no nicer path, which is itself worth knowing. In your
          browser:{' '}
          <em>
            DevTools → Application → Cookies →
            robertsspaceindustries.com → Rsi-Token
          </em>
          . Copy the value and paste it into the desktop app&apos;s
          settings.
        </p>
        <p style={{ color: 'var(--fg-muted)' }}>
          If a website ever asks you to do this, be suspicious. We ask
          because the app runs on your machine and the cookie never leaves
          it. A website asking for the same string is asking you to hand
          your session to a server.
        </p>
      </section>

      <section className="ss-about-section">
        <div className="ss-about-section-eyebrow">04 — How long it lasts</div>
        <h2>We don&apos;t know, and won&apos;t pretend to.</h2>
        <p>
          There&apos;s no fixed expiry we can quote. It stops working
          whenever RSI decides your session is over — a password change, a
          logout, or their own timing. When it lapses, hangar refresh
          starts failing and the app tells you it needs a new one. That
          isn&apos;t a bug; it&apos;s the cookie doing its job.
        </p>
      </section>

      <section className="ss-about-section">
        <div className="ss-about-section-eyebrow">05 — Taking it back</div>
        <h2>Clear cookie, in the app.</h2>
        <p>
          The settings pane has a <strong>Clear cookie</strong> control. It
          removes the cookie from your keychain, and it warns you what
          you&apos;re giving up:{' '}
          <em>
            &ldquo;Clear the stored RSI cookie? Hangar refresh will pause
            until you paste a new one.&rdquo;
          </em>
        </p>
        <p style={{ color: 'var(--fg-muted)' }}>
          That&apos;s the part we control, so it&apos;s the part we&apos;ll
          promise. What RSI does with the session on their side is theirs
          to answer, and we&apos;re not going to guess at it on your
          behalf.
        </p>
      </section>

      <section className="ss-about-section">
        <div className="ss-about-section-eyebrow">Related</div>
        <h2>The rest of the honest version.</h2>
        <p>
          <Link href="/trust">/trust</Link> covers what leaves your machine,
          what StarStats sees about other players, and why it can&apos;t get
          you banned. Back to{' '}
          <Link href={'/docs' as Route}>the quickstart</Link>.
        </p>
      </section>
    </main>
  );
}
