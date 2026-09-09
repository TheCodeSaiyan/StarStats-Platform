import { MarketingSurface } from '@/components/projection/MarketingSurface';
import { DocsIndex } from '@/components/projection/DocsIndex';
import type { Metadata, Route } from 'next';
import Link from 'next/link';

export const metadata: Metadata = {
  title: 'Troubleshooting',
  description:
    'No Game.log found, channel mismatch, sync refused, kills not tracked, a white window on Linux, and Test connection rejecting your host — what each one means, plus where the logs are.',
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
 *    0.85 confidence precisely because the branch is gone.
 *
 * 3. Section 07 documents a LIVE, UNFIXED bug — do not soften it into a
 *    workaround. An earlier version of this section blamed WebKitGTK's
 *    DMABUF renderer and told readers to set
 *    WEBKIT_DISABLE_DMABUF_RENDERER=1. That was wrong, and 0.1.14
 *    shipped a main.rs change on the strength of it that fixes nothing.
 *    The real failure is EGL display creation inside the bundled Ubuntu
 *    WebKit against newer Mesa, reproduced 2026-09-08 in a clean Arch
 *    container (Mesa 26.2.2) — so it is not GPU-, driver- or
 *    session-specific. Six env vars were measured against that repro and
 *    none help; removing the bundled WebKit only trades the blank window
 *    for a symbol-lookup crash in libavif/gstreamer. Do NOT re-add a
 *    suggested workaround here unless it has been watched to work.
 *
 * 4. Log paths in "Sending a log" come from
 *    `directories::ProjectDirs::from("app", "StarStats", "tray")`
 *    (config.rs:1227). On Linux the crate IGNORES qualifier and
 *    organization and uses the lowercased application name alone —
 *    hence `~/.local/share/tray`, not `.../StarStats/tray`. Don't
 *    "correct" it to the branded path. */
export default function TroubleshootingPage() {
  return (
    <MarketingSurface
      navId="docs"
      crumb={[
        { label: 'Site', href: '/' },
        { label: 'Docs', href: '/docs' },
        { label: 'Troubleshooting' },
      ]}
      title="Troubleshooting"
      ctx="Docs · when something is not arriving"
    >
      <DocsIndex active="/docs/troubleshooting" />
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
        Seven things account for most of it. Two of them aren&apos;t bugs and
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

      <section className="ss-about-section" id="linux-white-screen">
        <div className="ss-about-section-eyebrow">07 — Linux</div>
        <h2>The AppImage opens a white window and stays white.</h2>
        <p>
          The title bar draws, the tray icon works, and the inside of the
          window is a blank white rectangle. Start it from a terminal and
          the reason is there:
        </p>
        <p>
          <code>
            Could not create default EGL display: EGL_BAD_PARAMETER.
            Aborting...
          </code>
        </p>
        <p>
          The AppImage carries its own copy of WebKitGTK, built on Ubuntu,
          and runs it against whatever graphics drivers your machine has.
          On distros shipping a much newer Mesa — Arch and its family, and
          rolling releases generally — that WebKit cannot create an EGL
          display, so the process that paints the page exits before drawing
          anything. The rest of the app carries on, which is why the window
          and the tray icon survive an empty page.
        </p>
        <p>
          <strong>
            This is a known bug, it is ours, and there is no workaround
            yet.
          </strong>{' '}
          The environment variables that turn up in searches for
          similar-looking problems do not fix it:{' '}
          <code>WEBKIT_DISABLE_DMABUF_RENDERER</code> and{' '}
          <code>WEBKIT_DISABLE_COMPOSITING_MODE</code> were both measured
          against a reproduction of this exact failure and changed nothing,
          and neither did forcing software rendering or an X11 backend. The
          bundled WebKit is the cause, so the fix belongs in how the
          AppImage is built rather than in anything you can set.
        </p>
        <p style={{ color: 'var(--fg-muted)' }}>
          The <code>.deb</code> is unaffected — it uses the WebKitGTK your
          system already has, which by definition matches your drivers. If
          you are on a distro that cannot take a <code>.deb</code>, there is
          nothing that works today — <Link href={'/support' as Route}>say
          so on support</Link> and you will be told when there is.
        </p>
      </section>

      <section className="ss-about-section" id="logs">
        <div className="ss-about-section-eyebrow">Sending a log</div>
        <h2>Where the app writes things down.</h2>
        <p>
          On Linux everything lands in{' '}
          <code>~/.local/share/tray/</code> (or{' '}
          <code>$XDG_DATA_HOME/tray/</code> if you&apos;ve set that). On
          Windows it&apos;s{' '}
          <code>%APPDATA%\StarStats\tray\data\</code>. Two files matter:
        </p>
        <p>
          <code>panic.log</code> is always written — every crash appends to
          it, with the version that crashed. <code>client.log.YYYY-MM-DD</code>{' '}
          is the detailed one and is <strong>off by default</strong>; turn on{' '}
          <em>Debug logging</em> in Settings, restart, and reproduce the
          problem to fill it.
        </p>
        <p style={{ color: 'var(--fg-muted)' }}>
          The catch: if your window is blank you can&apos;t reach Settings to
          turn that on. Start the app from a terminal instead — it prints the
          same log to the terminal whether or not the file is enabled, and
          that output is the useful thing to paste into a{' '}
          <Link href={'/support' as Route}>bug report</Link>.
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
    </div>
    </MarketingSurface>
  );
}
