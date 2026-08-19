import type { Metadata, Route } from 'next';
import Link from 'next/link';
import { CompassStar } from '@/components/CompassStar';

export const metadata: Metadata = {
  title: 'About',
  description:
    'StarStats is a fan-built, local-first event tracker and manifest archive for Star Citizen players. Not affiliated with Cloud Imperium Games or Roberts Space Industries.',
};

// CIG fandom-guideline outbound links. These are required references on the
// About page per brand book §11; the live URLs are the canonical ones from
// docs/About.html and must match the standalone artifact byte-for-byte where
// the wording is "verbatim".
const COMPLIANCE_LINKS: ReadonlyArray<{
  href: string;
  label: string;
  note?: string;
}> = [
  {
    href: 'https://support.robertsspaceindustries.com/hc/en-us/articles/360006895793',
    label: 'Star Citizen Fankit and Fandom FAQ',
  },
  {
    href: 'https://support.robertsspaceindustries.com/hc/en-us/articles/115013196127',
    label: 'Fandom FAQ — Videos, writing, and more',
  },
  {
    href: 'https://support.robertsspaceindustries.com/hc/en-us/articles/5422808416151',
    label: 'Fan Film and Machinima Policy',
    note: 'for video creators; informs but does not govern this tool',
  },
  {
    href: 'https://robertsspaceindustries.com/en/fankit',
    label: 'RSI Fankit — download & terms',
  },
  {
    href: 'https://robertsspaceindustries.com/tos',
    label: 'RSI Terms of Service',
  },
];

export default function AboutPage() {
  return (
    <main className="ss-about">
      <div
        className="ss-about-lockup"
        aria-label="StarStats"
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 'var(--s3)',
          marginBottom: 'var(--s6)',
          color: 'var(--accent)',
        }}
      >
        <CompassStar size={40} label="StarStats compass star" />
        <span
          className="ss-wordmark"
          style={{
            fontSize: 'var(--fs-2xl)',
            fontWeight: 600,
            letterSpacing: '-0.035em',
            color: 'var(--fg)',
          }}
        >
          Star<em style={{ fontStyle: 'normal', color: 'var(--accent)' }}>Stats</em>
        </span>
      </div>

      <div className="ss-placard" style={{ marginBottom: 'var(--s5)' }}>
        About
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
        A pilot&apos;s logbook. Nothing more.
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
        StarStats is a fan-built, local-first event tracker and manifest archive
        for Star Citizen players. It pairs with the game, parses your raw log
        files on your own machine, and turns them into a clean, exportable
        session-by-session record you actually own.
      </p>

      <section className="ss-about-section">
        <div className="ss-about-section-eyebrow">01 — What it is</div>
        <h2>Made by a player, for players.</h2>
        <p>
          Every session, the game writes a log. StarStats reads it, sorts it,
          and gives you back a logbook — kills, deaths, cargo, contracts, jumps,
          ships flown, time on station — without uploading anything you
          didn&apos;t ask it to upload. Your manifest lives on your drive. Sync
          is opt-in. Export is always available, in NDJSON, ZIP, or CSV, so you
          can leave whenever you want.
        </p>
        <p style={{ color: 'var(--fg-muted)' }}>
          It does one job and tries to do it well. Pairing, parsing, viewing,
          and exporting are free — and will always be. There&apos;s a supporter
          tier with cosmetic recognition and longer retention, but no part of
          the core utility sits behind a paywall.
        </p>
      </section>

      <section className="ss-about-section">
        <div className="ss-about-section-eyebrow">02 — Made by</div>
        <h2>People, not a publisher.</h2>
        <div
          className="ss-credit"
          role="note"
          aria-label="Project credit"
        >
          <span
            style={{ color: 'var(--accent)', display: 'inline-flex' }}
            aria-hidden="true"
          >
            <CompassStar size={36} />
          </span>
          <div>
            <div className="ss-credit-meta">Builder</div>
            <div className="ss-credit-name">Nigel Tatschner</div>
            <div className="ss-credit-handle">
              @TheCodeSaiyan ·{' '}
              <a
                href="https://thecodesaiyan.io"
                target="_blank"
                rel="noopener noreferrer"
              >
                thecodesaiyan.io
              </a>
            </div>
          </div>
        </div>
        <p
          style={{
            marginTop: 'var(--s4)',
            fontSize: 'var(--fs-sm)',
            color: 'var(--fg-muted)',
          }}
        >
          StarStats is a community project. Contributions, bug reports, and
          pull requests are welcome on the{' '}
          <a
            href="https://github.com/TheCodeSaiyan/StarStats-Platform"
            target="_blank"
            rel="noopener noreferrer"
          >
            project&apos;s source repository
          </a>
          .
        </p>
      </section>

      <section className="ss-about-section" id="community-data-sources">
        <div className="ss-about-section-eyebrow">
          03 — Game reference data
        </div>
        <h2>The catalogue behind the names.</h2>
        <p>
          The names, manufacturers, roles, and sizes that StarStats uses to
          turn opaque engine identifiers like{' '}
          <code>AEGS_Avenger_Stalker</code> into &quot;Avenger
          Stalker&quot; are facts about the game — created by, and the
          property of, Cloud Imperium. StarStats bundles a compact,
          build-time snapshot of that catalogue (ship, vehicle, weapon,
          item, and location names plus their factual specifications and
          taxonomy) so the timeline reads like a logbook instead of a wall
          of engine symbols.
        </p>
        <p style={{ color: 'var(--fg-muted)' }}>
          <strong>Attribution.</strong> Star Citizen names, specifications,
          and taxonomy are © Cloud Imperium Rights LLC / Cloud Imperium
          Rights Ltd. StarStats is unofficial fan reference, not endorsed
          by or affiliated with Cloud Imperium Games or Roberts Space
          Industries. The credit here, in the footer of every surface, and
          in <code>NOTICE</code> exists so that attribution travels with
          the data.
        </p>
        <p style={{ color: 'var(--fg-muted)' }}>
          <strong>Facts only.</strong> The bundled catalogue is limited to
          factual data — class names, display names, slugs, per-category
          specifications, and location taxonomy (system, parent,
          classification). It deliberately carries{' '}
          <em>no descriptive prose</em>: any long-form flavour text is
          dropped when the snapshot is generated, so StarStats redistributes
          only uncopyrightable facts plus first-party Cloud Imperium data.
          The <Link href={'/kb' as Route}>/kb</Link> Knowledge Base pages
          render those facts (and, for vehicles, the official Ship Matrix
          specs described below).
        </p>
        <p style={{ color: 'var(--fg-muted)' }}>
          <strong>No warranty.</strong> The catalogue may be incomplete,
          out of date, or wrong about a given entity; StarStats passes the
          data through without warranties of accuracy, completeness, or
          fitness for purpose. Live in-game data can diverge from the
          bundled snapshot at any time.
        </p>
        <p style={{ color: 'var(--fg-muted)' }}>
          <strong>Ship specifications &amp; imagery.</strong> For vehicles,
          the spec sheets, flavour descriptions, and ship images shown on{' '}
          <Link href={'/kb' as Route}>Knowledge Base</Link> pages are
          enriched directly from Roberts Space Industries&apos; official{' '}
          <a
            href="https://robertsspaceindustries.com/ship-matrix"
            target="_blank"
            rel="noopener noreferrer"
          >
            Ship Matrix
          </a>
          . Those specifications, descriptions, and images are © Cloud
          Imperium Rights LLC / Cloud Imperium Rights Ltd and are shown
          here as unofficial fan reference, not endorsed by or affiliated
          with Cloud Imperium Games or Roberts Space Industries. Each
          vehicle page carries this notice inline, and we honour removal
          or adjustment requests — see the disclaimer in §04 below.
        </p>
      </section>

      {/*
        Required fan-fiction disclaimer. The text inside .ss-disclaimer-body
        is verbatim from brand book §11 and is mirrored in docs/About.html.
        Do not paraphrase it; keep the two surfaces byte-identical where the
        wording is required.
      */}
      <section className="ss-about-section">
        <div className="ss-about-section-eyebrow">
          04 — Compliance · fan-fiction disclaimer
        </div>
        <h2>The legal floor.</h2>
        <p>
          StarStats is a fan-built site and desktop tool, not a derivative
          work, mod, or replacement of any game. It is published as fan
          fiction and fan tooling under the spirit of Cloud Imperium&apos;s
          published fandom guidelines. The text below appears verbatim
          wherever it is required.
        </p>
        <aside className="ss-disclaimer" aria-label="Fan-fiction disclaimer">
          <div className="ss-disclaimer-label">
            Required — fan-fiction disclaimer (verbatim)
          </div>
          <p className="ss-disclaimer-body">
            StarStats is a work of fan fiction and fan tooling. All characters,
            places, events, ships, ship designs, and other content originating
            from Star Citizen, Squadron 42, or other content produced or
            created by their publishers or developers, are the property of
            Cloud Imperium Rights LLC and Cloud Imperium Rights Ltd. StarStats
            is unofficial, fan-made content not endorsed by or affiliated with
            Cloud Imperium Games or Roberts Space Industries.
          </p>
        </aside>
      </section>

      <section className="ss-about-section">
        <div className="ss-about-section-eyebrow">05 — Verify</div>
        <h2>Check our work.</h2>
        <p>
          We try to read CIG&apos;s fandom guidelines conservatively. If you
          want to confirm what they actually allow — and where StarStats sits
          within those boundaries — these are the source documents we work
          from.
        </p>
        <ul className="ss-resources">
          {COMPLIANCE_LINKS.map((link) => (
            <li key={link.href}>
              <a href={link.href} target="_blank" rel="noopener noreferrer">
                {link.label}
              </a>
              {link.note && (
                <span className="ss-resource-note"> — {link.note}</span>
              )}
            </li>
          ))}
        </ul>
      </section>

      <div
        className="ss-attribution-lockup"
        aria-label="Fan-made attribution"
      >
        <div className="ss-attribution-top">Fan-made · Not affiliated with</div>
        <div className="ss-attribution-mid">Cloud Imperium Games · RSI</div>
        <div className="ss-attribution-bot">
          Star Citizen™ &amp; Squadron 42™ are trademarks of CIG
        </div>
      </div>

      <p
        style={{
          marginTop: 'var(--s7)',
          fontSize: 'var(--fs-xs)',
          color: 'var(--fg-dim)',
        }}
      >
        <Link href="/" style={{ color: 'inherit' }}>
          ← Back home
        </Link>
      </p>
    </main>
  );
}
