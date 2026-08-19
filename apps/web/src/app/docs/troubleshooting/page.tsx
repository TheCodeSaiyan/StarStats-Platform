import type { Metadata, Route } from 'next';
import Link from 'next/link';

export const metadata: Metadata = {
  title: 'Troubleshooting',
  description:
    'No Game.log found, channel mismatch, sync refused, kills not tracked, and Test connection rejecting your host — what each one means.',
};

/* Every item here exists in code today; nothing is hypothetical.
 *
 * TWO traps, both load-bearing:
 *
 * 1. Channels. discovery.rs:56 scans FIVE (LIVE, PTU, EPTU, HOTFIX,
 *    TECH-PREVIEW). The tray hint says three ("Leave blank to
 *    auto-discover the largest LIVE/PTU/EPTU log", SettingsPane.tsx:816),
 *    and that drift is repeated at api.ts:390 and commands.rs:498. Quote
 *    the hint as UI TEXT; never repeat it as the channel list, or this
 *    page goes wrong for HOTFIX and TECH-PREVIEW users.
 *
 * 2. Kills. CIG REMOVED the `<Actor Death>` line (events.rs:191-195). It
 *    is NOT CVar-gated — there is no verbosity that brings it back, and
 *    telling users to enable CVars is false advice. inference.rs:371-384
 *    synthesizes PlayerDeath from VehicleDestruction + ResolveSpawn at
 *    0.85 confidence precisely because the branch is gone. */
export default function TroubleshootingPage() {
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
        When it isn&apos;t working.
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
        Six things account for most of it. Two of them aren&apos;t bugs and
        never will be — the first one leads, because knowing that saves you
        the hour.
      </p>

      <section className="ss-about-section" id="kills">
        <div className="ss-about-section-eyebrow">01 — Not a bug</div>
        <h2>Your kills aren&apos;t tracked. They can&apos;t be.</h2>
        <p>
          Star Citizen used to write a line naming who killed whom. CIG
          removed it. It isn&apos;t hidden behind a setting — there is no
          logging option that brings it back, and anyone telling you to
          enable one is guessing.
        </p>
        <p>
          So StarStats works it out sideways: your ship is destroyed, you
          respawn, and it infers that you died — marked as inferred,
          because it is. Your own deaths come through reliably. Kill credit
          against another player does not, and won&apos;t until the game
          logs it again.
        </p>
      </section>

      <section className="ss-about-section" id="no-log">
        <div className="ss-about-section-eyebrow">02 — No log found</div>
        <h2>
          &ldquo;No Game.log found — set a path in Settings to start the
          feed.&rdquo;
        </h2>
        <p>
          The app looks for your game folder on its own and usually finds
          it. When it can&apos;t, point it at the file by hand in Settings.
        </p>
        <p>
          The hint by that box says{' '}
          <em>
            &ldquo;Leave blank to auto-discover the largest LIVE/PTU/EPTU
            log&rdquo;
          </em>{' '}
          — that text is out of date and undersells it. Auto-discovery
          actually walks five channels: LIVE, PTU, EPTU, HOTFIX and
          TECH-PREVIEW. If you play on HOTFIX or TECH-PREVIEW, leaving it
          blank still works.
        </p>
      </section>

      <section className="ss-about-section" id="channel-mismatch">
        <div className="ss-about-section-eyebrow">03 — Channel mismatch</div>
        <h2>Running one build, updating from another.</h2>
        <p>
          The banner means your installed build and your update channel
          disagree — a beta build set to take stable updates, or the
          reverse. The next update check will poll the channel you
          configured, which may not be the one you&apos;re running. Set the
          release channel to match the build you want to stay on.
        </p>
      </section>

      <section className="ss-about-section" id="sync-refused">
        <div className="ss-about-section-eyebrow">04 — Sync refused</div>
        <h2>
          &ldquo;This uplink&apos;s sync is disabled.&rdquo;
        </h2>
        <p>
          The app is sending and the server is refusing, because cloud sync
          is off for that uplink. Turn it on from Connected Uplinks on the
          web, or tick <strong>Sync settings with your account</strong> in
          the app and press Save.
        </p>
        <p style={{ color: 'var(--fg-muted)' }}>
          Worth saying: this is not you being logged out. It reads like an
          auth error and isn&apos;t one — an older build made exactly that
          mistake and unpaired itself in a loop over it.
        </p>
      </section>

      <section className="ss-about-section" id="test-connection">
        <div className="ss-about-section-eyebrow">05 — Test connection</div>
        <h2>
          &ldquo;URL targets a private/loopback host.&rdquo;
        </h2>
        <p>
          Pointing Test connection at <code>localhost</code>, a{' '}
          <code>192.168.</code>/<code>10.</code>/<code>172.16.</code>{' '}
          address, or a link-local one is refused on purpose. It stops the
          app being talked into probing machines inside your network.
        </p>
        <p style={{ color: 'var(--fg-muted)' }}>
          If you&apos;re self-hosting, this is the one that&apos;ll bite
          you, and it&apos;s working as designed rather than failing.
        </p>
      </section>

      <section className="ss-about-section" id="cookie-lapsed">
        <div className="ss-about-section-eyebrow">06 — Hangar stopped</div>
        <h2>The cookie went stale.</h2>
        <p>
          Hangar data comes from your RSI session cookie, and that lapses
          whenever RSI ends the session. Paste a fresh one — see{' '}
          <Link href={'/docs/rsi-cookie' as Route}>the cookie page</Link>{' '}
          for where to find it.
        </p>
      </section>

      <section className="ss-about-section">
        <div className="ss-about-section-eyebrow">Still stuck?</div>
        <h2>Tell us — that&apos;s what the beta is for.</h2>
        <p>
          If none of this fits, it&apos;s worth reporting: during the beta
          a parser gap is the likeliest cause, and those only get fixed
          when someone files them. Back to{' '}
          <Link href={'/docs' as Route}>the quickstart</Link>.
        </p>
      </section>
    </main>
  );
}
