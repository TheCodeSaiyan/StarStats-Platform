import type { Metadata, Route } from 'next';
import Link from 'next/link';
import { CODE_SIGNING_PUBLISHER } from '@/lib/signing';

export const metadata: Metadata = {
  title: 'Docs',
  description:
    'Install StarStats, pair your desktop app, add your RSI cookie, verify your handle, and turn on sync — the five steps, in order. Then the usage guides for the desktop app and the website.',
};

/* Instruction, not persuasion — /features argues, this page tells you
 * which button. Reuses the /about + /trust section furniture (eyebrow +
 * h2 plateau) so the three read as one voice.
 *
 * Ordering is the drop-off order, not the tidy order: install, pair,
 * cookie, verify, sync. Four of those five are steps a stranger does not
 * guess, and each is a silent drop-off today.
 *
 * Facts here are traced, not remembered — see the fact table in
 * the release design notesplans/2026-07-17-docs-onboarding.md. Three premises
 * were wrong when checked: pairing is web->tray (NOT tray->web),
 * `redeem_pair` does not exist (it is `pair_device` / server `redeem`,
 * and the old name survives in five stale comments), and releases ship
 * FOUR bundle targets, not two. */
export default function DocsPage() {
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
        Get StarStats running.
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
        Five steps, in the order people get stuck on them. Four of them
        you would not guess. If something here doesn&apos;t match what
        you&apos;re seeing, that&apos;s a bug in this page — tell us.
      </p>

      <section className="ss-about-section" id="install">
        <div className="ss-about-section-eyebrow">01 — Install</div>
        <h2>Windows or Linux. There is no macOS build.</h2>
        <p>
          Saying that plainly beats letting you find out after a download.
          Grab the latest from{' '}
          <Link href="/downloads">the downloads page</Link>
          . On Windows take the installer — <code>_x64-setup.exe</code> —
          or the <code>.msi</code> if your setup prefers it. On Linux
          there&apos;s an <code>_amd64.AppImage</code> and a{' '}
          <code>.deb</code>.
        </p>
        <p style={{ color: 'var(--fg-muted)' }}>
          The Windows installers are signed with a CA-issued certificate, so
          the signature names <strong>{CODE_SIGNING_PUBLISHER}</strong> rather
          than an unknown publisher. SmartScreen may still prompt for a while
          — that&apos;s a separate reputation system, and a newly-issued
          signing identity starts with none. Choose{' '}
          <strong>More info</strong> → <strong>Run anyway</strong>, or check
          the signature first —{' '}
          <Link href="/downloads">the downloads page</Link> shows exactly what
          it should say.
        </p>
        <p style={{ color: 'var(--fg-muted)' }}>
          The desktop app is the thing that reads your log. Everything
          else on this page is about connecting it to your account.
        </p>
      </section>

      <section className="ss-about-section" id="pair">
        <div className="ss-about-section-eyebrow">02 — Pair</div>
        <h2>The website makes the code. The app takes it.</h2>
        <p>
          That&apos;s the way round people get wrong, so: on the web, go to
          Connected Uplinks and press <strong>Generate pairing code</strong>.
          You get eight characters. Type them into the desktop app.
        </p>
        <p>
          The code lasts <strong>five minutes</strong> and works once. It
          skips letters that look like other letters — no I, L, O, 1 or 0 —
          so you can read it off the screen without squinting.
        </p>
        <p style={{ color: 'var(--fg-muted)' }}>
          The code isn&apos;t a secret worth guarding: it only works for
          the account that generated it, so someone shoulder-surfing it
          can&apos;t attach their own machine to you.
        </p>
      </section>

      <section className="ss-about-section" id="cookie">
        <div className="ss-about-section-eyebrow">03 — RSI cookie</div>
        <h2>Optional. This is the one to read about first.</h2>
        <p>
          To show your hangar, StarStats reads your RSI profile using your
          own login — which means pasting a session cookie named{' '}
          <code>Rsi-Token</code> into the desktop app. Skip it and
          everything else still works; you just don&apos;t get hangar data.
        </p>
        <p>
          It&apos;s worth understanding what that grants before you paste
          it, so it has{' '}
          <Link href={'/docs/rsi-cookie' as Route}>its own page</Link>.
        </p>
      </section>

      <section className="ss-about-section" id="verify">
        <div className="ss-about-section-eyebrow">04 — Verify your handle</div>
        <h2>Prove the handle is yours, via your RSI bio.</h2>
        <p>
          On <Link href={'/settings' as Route}>/settings</Link>, start
          verification. You get a code shaped like{' '}
          <code>STARSTATS-XXXXXXXX</code>. Paste it into your{' '}
          <strong>public RSI bio</strong>, save the bio on RSI, then come
          back and press <strong>Check now</strong>. We read your public
          profile page and look for the code.
        </p>
        <p>
          The code lasts <strong>30 minutes</strong>. Nothing polls — the
          check only happens when you press the button, so saving the bio
          and walking away won&apos;t verify you.
        </p>
        <p style={{ color: 'var(--fg-muted)' }}>
          Refreshing the page hands you the same code back rather than a
          new one, so you don&apos;t have to re-paste it every time you
          lose your place.
        </p>
      </section>

      <section className="ss-about-section" id="sync">
        <div className="ss-about-section-eyebrow">05 — Sync</div>
        <h2>Off until you turn it on. Then press Save.</h2>
        <p>
          In the desktop app&apos;s settings there&apos;s a checkbox:{' '}
          <strong>Sync settings with your account</strong>. It&apos;s off
          by default, and it stays greyed out until the app is paired —
          step 02. Ticking it isn&apos;t enough on its own; the settings
          pane holds your changes until you hit <strong>Save</strong>.
        </p>
        <p>
          With it on, this app&apos;s theme and settings live on your
          account and follow you to other machines. You can revoke that
          from Connected Uplinks on the web at any time.
        </p>
        <p style={{ color: 'var(--fg-muted)' }}>
          What this checkbox does <em>not</em> mean is &ldquo;nothing
          leaves your PC until now&rdquo; — a paired app sends, and the
          server refuses what it shouldn&apos;t have. Both halves are true
          and <Link href="/trust">/trust</Link> explains them.
        </p>
      </section>

      <section className="ss-about-section" id="using">
        <div className="ss-about-section-eyebrow">Running? Now use it</div>
        <h2>That&apos;s setup done. Guides take it from here.</h2>
        <p>
          <Link href={'/guides' as Route}>Guides</Link> is the other
          section, and it assumes what&apos;s above already works:{' '}
          <Link href={'/guides/desktop-app' as Route}>the desktop app</Link>{' '}
          (four of its six tabs aren&apos;t named after their job),{' '}
          <Link href={'/guides/dashboard' as Route}>your dashboard</Link>,{' '}
          <Link href={'/guides/sharing' as Route}>sharing</Link> and{' '}
          <Link href={'/guides/settings' as Route}>settings</Link>.
        </p>
      </section>

      <section className="ss-about-section">
        <div className="ss-about-section-eyebrow">Stuck?</div>
        <h2>Most of it is one of six things.</h2>
        <p>
          No log found, the wrong game channel, sync being refused, kills
          not showing up —{' '}
          <Link href={'/docs/troubleshooting' as Route}>
            troubleshooting
          </Link>{' '}
          covers each one and why it happens.
        </p>
      </section>
    </main>
  );
}
