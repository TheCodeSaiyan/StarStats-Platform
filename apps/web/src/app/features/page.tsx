import { MarketingSurface } from '@/components/projection/MarketingSurface';
import Image from 'next/image';
import Link from 'next/link';
import type { Metadata } from 'next';
import { redirect } from 'next/navigation';

import { getSession } from '@/lib/session';

export const metadata: Metadata = {
  // No brand suffix here: layout.tsx's title.template appends " — StarStats".
  // openGraph.title below keeps it, since a social card stands alone.
  title: 'Features',
  description:
    'StarStats reads what the game already writes and turns it into a timeline you can actually look at. Local-first, EAC-safe, per-device cloud sync.',
  openGraph: {
    title: 'Features · StarStats',
    description:
      'StarStats reads what the game already writes and turns it into a timeline you can actually look at.',
    images: [{ url: '/social/og.png', width: 1200, height: 630 }],
  },
};

/**
 * Marketing features page. Mirrors the eyebrow + h2 layout convention
 * from `/about` (numbered sections, monospace eyebrow + h2 plateau)
 * and the design tokens from globals.css. The hero is a live
 * comparison — raw Game.log text on the left, rendered StarStats
 * event cards on the right — so the "after" side stays in sync with
 * the actual product styling instead of drifting from a screenshot.
 */
export default async function FeaturesPage() {
  // Marketing pages redirect signed-in users into the app shell so
  // they don't waste a click — same pattern as `/`.
  const session = await getSession();
  if (session) redirect('/me');

  return (
    <MarketingSurface
      navId="features"
      crumb={[
        { label: 'Site', href: '/' },
        { label: 'Features' },
      ]}
      title="Features"
      ctx="What StarStats does, in detail"
    >
    <div className="ss-landing" style={{ minHeight: '100%', position: 'relative' }}>
      <div style={{ position: 'relative', zIndex: 1, maxWidth: 'none', margin: 0, padding: 0 }}>
        <HeroComparison />
        <SectionTransparency />
        <SectionDashboard />
        <SectionKnowledge />
        <SectionCloudSync />
        <SectionRecords />
        <SectionSharing />
        <SectionOwnership />
        <Footer />
      </div>
    </div>
    </MarketingSurface>
  );
}

// -- Hero: "Go from this to this" comparison --------------------------

function HeroComparison() {
  return (
    <section
      style={{
        padding: '96px 48px 56px',
        maxWidth: 1280,
        margin: '0 auto',
      }}
    >
      <div style={{ maxWidth: 720 }}>
        <span
          className="ss-placard"
          style={{ color: 'var(--fg-dim)', display: 'inline-block', marginBottom: 12 }}
        >
          What StarStats does
        </span>
        <h1
          style={{
            margin: 0,
            fontSize: 'clamp(40px, 6vw, 64px)',
            fontWeight: 600,
            letterSpacing: 'var(--tracking-tight)',
            lineHeight: 1.05,
          }}
        >
          Go from this
          <span style={{ color: 'var(--fg-dim)' }}>…</span>
          <br />
          to this.
        </h1>
        <p
          style={{
            margin: '20px 0 0',
            maxWidth: '60ch',
            color: 'var(--fg-muted)',
            fontSize: 'var(--fs-lg)',
            lineHeight: 1.55,
          }}
        >
          The game already writes everything you did to a log file. It just doesn&apos;t look like
          much. StarStats reads it locally, parses it, and renders the parts you&apos;d actually
          want to see.
        </p>
      </div>

      <div
        style={{
          marginTop: 48,
          display: 'grid',
          gridTemplateColumns: 'minmax(0, 1fr) auto minmax(0, 1fr)',
          gap: 24,
          alignItems: 'stretch',
        }}
      >
        <BeforePane />
        <Arrow />
        <AfterPane />
      </div>

      <p
        style={{
          margin: '28px auto 0',
          maxWidth: '64ch',
          color: 'var(--fg-dim)',
          fontSize: 13,
          lineHeight: 1.55,
          textAlign: 'center',
        }}
      >
        Real fragment of <code className="mono">Game.log</code>. Every line is something the game
        wrote about itself. StarStats turns the ones that matter to <em>you</em> into a timeline.
      </p>
    </section>
  );
}

function BeforePane() {
  // Real screenshot of a Star Citizen Game.log open in a plain text
  // editor. Public output the game ships to disk for every player —
  // dense, technical, low-signal-to-noise. The contrast with the
  // rendered AfterPane is the whole point of this hero.
  return (
    <div
      className="ss-card"
      style={{
        padding: 'var(--s3) var(--s4)',
        background: 'var(--bg-elev)',
        border: '1px solid var(--border)',
        overflow: 'hidden',
        display: 'flex',
        flexDirection: 'column',
      }}
    >
      <div
        className="ss-placard"
        style={{
          color: 'var(--fg-dim)',
          marginBottom: 10,
          fontSize: 11,
        }}
      >
        Game.log · raw
      </div>
      <div
        style={{
          position: 'relative',
          flex: 1,
          minHeight: 320,
          borderRadius: 0,
          overflow: 'hidden',
          border: '1px solid var(--border-dim)',
        }}
      >
        <Image
          src="/features/game-log-raw.png"
          alt="A real excerpt of a Star Citizen Game.log file open in a plain text editor. Dense lines of timestamps, engine subsystem tags, JSON-like payloads, and trace IDs — what the game writes about itself before StarStats parses it."
          width={1133}
          height={746}
          priority
          style={{
            display: 'block',
            width: '100%',
            height: 'auto',
            objectFit: 'cover',
            objectPosition: 'top left',
          }}
        />
      </div>
      <div
        style={{
          marginTop: 10,
          paddingTop: 10,
          borderTop: '1px solid var(--border)',
          fontSize: 11,
          color: 'var(--fg-dim)',
          fontFamily: 'var(--font-mono)',
        }}
      >
        149,073 chars · one play session
      </div>
    </div>
  );
}

function AfterPane() {
  // The same 12 raw lines collapse to one human-readable event from
  // the player's perspective: a spawn into the frontend lobby. The
  // rest is engine bookkeeping StarStats filters out at the parser.
  // Three additional events sit around it for context — the kind of
  // surface you'd actually see on /dashboard.
  const events: ReadonlyArray<{
    glyph: string;
    title: string;
    detail: string;
    when: string;
    tone: 'accent' | 'ok' | 'muted';
  }> = [
    {
      glyph: '⬢',
      title: 'Session started',
      detail: 'Game.log opened · LIVE channel',
      when: '16:54:28',
      tone: 'muted',
    },
    {
      glyph: '◉',
      title: 'Spawned into Stanton',
      detail: 'megamap · SC_Frontend lobby',
      when: '16:54:29',
      tone: 'accent',
    },
    {
      glyph: '↗',
      title: 'Joined Persistent Universe',
      detail: 'shard 8c4eb8a6 · player bound',
      when: '16:54:30',
      tone: 'ok',
    },
    {
      glyph: '⤓',
      title: 'Streaming bubble ready',
      detail: 'always-streamed entities online',
      when: '16:54:31',
      tone: 'muted',
    },
  ];

  const toneColor = (tone: 'accent' | 'ok' | 'muted') =>
    tone === 'accent' ? 'var(--accent)' : tone === 'ok' ? 'var(--ok)' : 'var(--fg-dim)';

  return (
    <div
      className="ss-card"
      style={{
        padding: 'var(--s3) var(--s4)',
        background: 'var(--bg-elev)',
        border: '1px solid var(--border)',
        display: 'flex',
        flexDirection: 'column',
      }}
    >
      <div
        className="ss-placard"
        style={{
          color: 'var(--fg-dim)',
          marginBottom: 10,
          fontSize: 11,
        }}
      >
        Your timeline · rendered
      </div>
      <ol
        aria-label="Rendered events from StarStats"
        style={{
          listStyle: 'none',
          margin: 0,
          padding: 0,
          display: 'flex',
          flexDirection: 'column',
          gap: 0,
          flex: 1,
        }}
      >
        {events.map((e, i) => (
          <li
            key={e.title}
            style={{
              display: 'grid',
              gridTemplateColumns: '20px 64px 1fr',
              gap: 10,
              alignItems: 'baseline',
              padding: '12px 4px',
              borderTop: i === 0 ? 'none' : '1px solid var(--border)',
              borderLeft: `2px solid ${toneColor(e.tone)}`,
              marginLeft: 2,
              paddingLeft: 12,
            }}
          >
            <span aria-hidden="true" style={{ color: toneColor(e.tone), fontSize: 14 }}>
              {e.glyph}
            </span>
            <time
              className="mono"
              style={{ color: 'var(--fg-dim)', fontSize: 11 }}
            >
              {e.when}
            </time>
            <div>
              <div style={{ color: 'var(--fg)', fontSize: 13, fontWeight: 500 }}>{e.title}</div>
              <div style={{ color: 'var(--fg-muted)', fontSize: 12, marginTop: 2 }}>
                {e.detail}
              </div>
            </div>
          </li>
        ))}
      </ol>
      <div
        style={{
          marginTop: 10,
          paddingTop: 10,
          borderTop: '1px solid var(--border)',
          fontSize: 11,
          color: 'var(--fg-dim)',
          fontFamily: 'var(--font-mono)',
        }}
      >
        4 events · engine noise filtered
      </div>
    </div>
  );
}

function Arrow() {
  return (
    <div
      aria-hidden="true"
      style={{
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
        color: 'var(--accent)',
        fontSize: 28,
        fontFamily: 'var(--font-mono)',
        userSelect: 'none',
      }}
    >
      →
    </div>
  );
}

// -- Section building blocks -------------------------------------------

function Section({
  number,
  eyebrow,
  title,
  lede,
  children,
}: {
  number: string;
  eyebrow: string;
  title: string;
  lede: string;
  children?: React.ReactNode;
}) {
  return (
    <section
      style={{
        maxWidth: 1080,
        margin: '0 auto',
        padding: 'var(--s8) var(--s7)',
        borderTop: '1px solid var(--border)',
      }}
    >
      <div style={{ display: 'flex', alignItems: 'baseline', gap: 14, marginBottom: 14 }}>
        <span
          className="mono"
          style={{
            color: 'var(--fg-dim)',
            fontSize: 12,
            letterSpacing: '0.06em',
          }}
        >
          {number}
        </span>
        <span
          className="ss-placard"
          style={{ color: 'var(--fg-dim)' }}
        >
          {eyebrow}
        </span>
      </div>
      <h2
        style={{
          margin: 0,
          fontSize: 'clamp(28px, 4vw, 40px)',
          fontWeight: 600,
          letterSpacing: '-0.02em',
          lineHeight: 1.1,
          maxWidth: '22ch',
        }}
      >
        {title}
      </h2>
      <p
        style={{
          margin: 'var(--s4) 0 0',
          maxWidth: '60ch',
          color: 'var(--fg-muted)',
          fontSize: 'var(--fs-md)',
          lineHeight: 1.6,
        }}
      >
        {lede}
      </p>
      {children}
    </section>
  );
}

function PointGrid({
  points,
}: {
  points: ReadonlyArray<{ title: string; body: React.ReactNode }>;
}) {
  return (
    <div
      style={{
        marginTop: 28,
        display: 'grid',
        gridTemplateColumns: 'repeat(auto-fit, minmax(240px, 1fr))',
        gap: 16,
      }}
    >
      {points.map((p) => (
        <div
          key={p.title}
          className="ss-card"
          style={{ padding: 'var(--s4) var(--s5)' }}
        >
          <div style={{ fontSize: 14, fontWeight: 600, marginBottom: 6, color: 'var(--fg)' }}>
            {p.title}
          </div>
          <div style={{ fontSize: 13, color: 'var(--fg-muted)', lineHeight: 1.6 }}>{p.body}</div>
        </div>
      ))}
    </div>
  );
}

// -- Sections ---------------------------------------------------------

function SectionTransparency() {
  return (
    <Section
      number="01"
      eyebrow="Data transparency"
      title="Reads what the game writes, never anything else."
      lede="No injection, no memory hooks, no scraping inside the client. The tray reads the same Game.log Star Citizen ships to disk for everyone, parses the lines that describe what you did, and ignores the rest."
    >
      <PointGrid
        points={[
          {
            title: 'EAC-safe by construction',
            body: 'Plain file reads from your own machine. Nothing touches the game process. Nothing your anti-cheat would object to.',
          },
          {
            title: 'Your events only',
            body: "Logins, deaths, missions, jumps, hangar refreshes — events the game already attributes to you. Never chat, never other players' positions.",
          },
          {
            title: 'Local-first by default',
            body: "Everything stays on your PC until you sign in and turn on remote sync. You're in charge of when, what, and how much.",
          },
        ]}
      />
    </Section>
  );
}

function SectionDashboard() {
  return (
    <Section
      number="02"
      eyebrow="The dashboard"
      title="A cockpit for your own play."
      lede="The StarStats dashboard reads like ship instrumentation: dense, legible, diegetic. A configurable grid of live panes — corner-bracketed where the data streams — with every figure in a mono readout and a telemetry rail down the side."
    >
      <PointGrid
        points={[
          {
            title: 'Live panes, earned brackets',
            body: 'The activity heatmap, travel, combat and economy readouts stream in real time — and carry corner brackets to say so. Snapshot panes stay calm. Brackets mean live, never decoration.',
          },
          {
            title: 'Telemetry rail',
            body: 'A persistent strip beside the nav streaming your current location, total events and locations visited — bracketed frames that collapse to a ticker on mobile.',
          },
          {
            title: 'Four themes, one layout',
            body: 'Stanton, Pyro, Terra and Nyx — warm amber, molten coral, clinical teal, and a light violet. Colour changes; the dense instrument structure never does. AA-legible in all four.',
          },
          {
            title: 'Focus lenses & ranges',
            body: 'Filter the whole board to Activity, Travel, Combat, Loadout or Commerce, and retime it from 24 hours to all-time. Drag to rearrange, resize, or hide any pane.',
          },
        ]}
      />
    </Section>
  );
}

function SectionKnowledge() {
  return (
    <Section
      number="03"
      eyebrow="Knowledge base"
      title="Every ship, item and place — named."
      lede="Engine identifiers like ORIG_100i are meaningless on their own. A wiki-synced catalogue turns them into real names across the whole app — with hover stats, full Ship Matrix spec sheets, a browseable contract catalogue, and a body-outline view of your loadout."
    >
      <PointGrid
        points={[
          {
            title: 'Named everywhere',
            body: 'Ships, weapons, items and locations resolve to their real names on every surface — timeline, sharing, org views — each with a hover card carrying the key stats.',
          },
          {
            title: 'Ship Matrix specs',
            body: 'Open any vehicle for the official RSI Ship Matrix sheet — dimensions, speed, crew, cargo — plus a peer-relative visual comparison against a cohort you pick.',
          },
          {
            title: 'Loadout paperdoll',
            body: 'The gear the game last restored, drawn on a body outline: armour by slot, weapons and carried kit grouped, named, and linked back to the catalogue.',
          },
          {
            title: 'Locations, classified',
            body: 'Systems, planets, moons, cities, stations and jump points sorted into an eight-tier hierarchy, so "you are here" always resolves to something real.',
          },
        ]}
      />
    </Section>
  );
}

function SectionCloudSync() {
  return (
    <Section
      number="04"
      eyebrow="Cloud sync"
      title="Sync your settings across devices — when you choose."
      lede="Pair the tray to your account once, then opt in to cloud sync per device. Your theme, sync cadence, and tray preferences ride along to every uplink you turn on. Off by default. You revoke any uplink remotely from the web."
    >
      <PointGrid
        points={[
          {
            title: 'Per-device opt-in',
            body: 'Cloud sync is off by default on every device. Flip the toggle on the tray you actually want to sync. The others stay strictly local.',
          },
          {
            title: 'Connected Uplinks page',
            body: "See every paired tray under one roof on /devices. Toggle sync on or off per uplink from the web — the device picks it up on its next tick.",
          },
          {
            title: 'Web is always your account view',
            body: 'When you sign in on a new browser, your theme follows from your account automatically. Clearing site data doesn’t lose your settings.',
          },
          {
            title: 'Last-write-wins',
            body: 'Change a setting on the tray, change it on the web — most recent write wins. No merge dialogs, no surprises. Each device only writes what it owns.',
          },
        ]}
      />
    </Section>
  );
}

function SectionRecords() {
  return (
    <Section
      number="05"
      eyebrow="Records & insights"
      title="Surface what actually stood out."
      lede="Your timeline is the raw material — the dashboard pulls out the moments worth telling someone about. The records widget highlights your deadliest single session and links straight back to the events behind it."
    >
      <PointGrid
        points={[
          {
            title: 'Deadliest session',
            body: 'The play period with the most kills and deaths, surfaced on your dashboard with a click-through to the full session timeline.',
          },
          {
            title: 'Activity heatmap',
            body: 'Per-day grid going back as far as your manifest. Pick a range from 30 days to a full year; one cell per day, one click to drill in.',
          },
          {
            title: 'Top event types',
            body: 'Sorted distribution of what you actually did. Filter the stream by type from the same chart — no separate query language to learn.',
          },
          {
            title: 'In-game manifest',
            body: 'Recent stops chain, latest pledge snapshot, RSI org membership. Everything ties back to a verifiable event in the log.',
          },
        ]}
      />
    </Section>
  );
}

function SectionSharing() {
  return (
    <Section
      number="06"
      eyebrow="Sharing, orgs & StarPlatform"
      title="Show the parts you choose. Hide the rest."
      lede="Profile-level visibility (public, RSI org, named handles, fully private) plus per-event scopes on top. Allow or deny individual event types per share. Verify a handle is yours by pasting a short code into your RSI bio for a minute."
    >
      <PointGrid
        points={[
          {
            title: 'Per-event visibility',
            body: 'Allow- or deny-list specific event types on each share. Show your deaths but not your hangar. Show your logins to your org but nothing else.',
          },
          {
            title: 'Named-handle grants',
            body: 'Grant access to a specific RSI handle with an optional expiry. They see what you share; they never see what they were never granted.',
          },
          {
            title: 'Orgs & StarPlatform',
            body: 'RSI org owners get a shared dashboard with ReBAC roles. Run a whole group live on the self-hosted StarPlatform companion — presence, roster and an ops board, opt-in on both ends.',
          },
          {
            title: 'Hash-chained audit',
            body: 'Every share action (created, viewed, revoked, reported, visibility-changed, device-sync toggled) lands in an append-only audit log you can inspect.',
          },
        ]}
      />
    </Section>
  );
}

function SectionOwnership() {
  return (
    <Section
      number="07"
      eyebrow="Export & ownership"
      title="Your numbers stay your file."
      lede="The whole manifest is yours. Download it, take it with you, delete it whenever. Nothing locks you in. Open formats, plain rows, no proprietary blobs."
    >
      <PointGrid
        points={[
          {
            title: 'Full manifest download',
            body: 'Per-day heatmap, top activities, full timeline, snapshot history. Whatever’s on the server about you, in one file.',
          },
          {
            title: 'Open formats',
            body: 'NDJSON for the timeline, CSV for tables, ZIP bundle for everything. Nothing you can’t open in a text editor.',
          },
          {
            title: 'Delete on request',
            body: 'Account deletion clears everything except the audit-chain tombstone that records the deletion itself. No silent retention.',
          },
          {
            title: 'Source-available',
            body: (
              <>
                Read the code on{' '}
                <a
                  href="https://github.com/TheCodeSaiyan/StarStats-Platform"
                  target="_blank"
                  rel="noreferrer noopener"
                  style={{ color: 'var(--accent)' }}
                >
                  GitHub
                </a>
                . If something looks off, raise an issue — or build it differently yourself.
              </>
            ),
          },
        ]}
      />
    </Section>
  );
}

// -- Footer -----------------------------------------------------------

function Footer() {
  return (
    // Full-bleed outer / capped inner (L4): the border-top must span the
    // viewport, so it lives on the un-capped <footer>; the max-width
    // lives on the inner wrapper. Putting max-width on the <footer>
    // itself painted the border as a centered 1080px stripe on wide
    // screens (the documented stripe-in-the-middle mistake).
    <footer style={{ borderTop: '1px solid var(--border)' }}>
      <div
        style={{
          maxWidth: 1080,
          margin: '0 auto',
          padding: '80px 48px 64px',
          display: 'flex',
          gap: 32,
          alignItems: 'flex-start',
          flexWrap: 'wrap',
        }}
      >
        <div style={{ flex: '1 1 320px', minWidth: 0 }}>
          <h2
            style={{
              margin: 0,
              fontSize: 'clamp(24px, 3vw, 32px)',
              fontWeight: 600,
              letterSpacing: 'var(--tracking-tight)',
            }}
          >
            See what your log says about you.
          </h2>
          <p
            style={{
              margin: '12px 0 0',
              color: 'var(--fg-muted)',
              fontSize: 15,
              lineHeight: 1.55,
              maxWidth: '54ch',
            }}
          >
            Sign up, pair the tray, play. The first events land in your timeline within seconds.
          </p>
        </div>
        <div style={{ display: 'flex', gap: 12, alignItems: 'center', flexWrap: 'wrap' }}>
          <Link href="/auth/signup" className="ss-btn ss-btn--primary">
            Get started →
          </Link>
          <Link href="/downloads" className="ss-btn ss-btn--ghost">
            Download
          </Link>
        </div>
      </div>
    </footer>
  );
}
