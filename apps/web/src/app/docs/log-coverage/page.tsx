import { MarketingSurface } from '@/components/projection/MarketingSurface';
import { DocsIndex } from '@/components/projection/DocsIndex';
import type { Metadata, Route } from 'next';
import Link from 'next/link';

export const metadata: Metadata = {
  title: 'What the logs say',
  description:
    'Which lines StarStats reads out of Game.log, which ones the game stopped writing, and which ones have never appeared at all.',
};

/* The companion to /docs/rsi-cookie, and written to the same rule: name
 * the uncomfortable thing first, then the control.
 *
 * WHY THIS PAGE EXISTS. Figures on this site have read zero for
 * reasons that had nothing to do with the reader's play — a line CIG
 * retired, a regex with a missing space. Each looked identical from
 * the outside: a confident 0. A reader cannot tell those apart, and
 * until this page there was nowhere that told them.
 *
 * NO PER-EVENT TABLE of all thirty-six types, deliberately. The list
 * is in `metadata.rs::event_type_key` and changes with the parser; a
 * copy of it here would go stale silently while reading as a
 * contract. The groups below are stable in a way the enum is not.
 *
 * WHAT IS OMITTED, AND WHY IT HAD TO BE. `game_crash` and
 * `launcher_activity` are parsed types that produce nothing today —
 * the crash scanner walks a `Crashes/` directory that is not on a
 * current install, and the launcher log changed to a JSON shape the
 * bracket-format reader in `launcher.rs` cannot match. Naming them as
 * things we read would be false, and this page is not the place that
 * discusses them, so they are named in neither list. If either is
 * ever fixed it belongs in section 02; while broken it belongs
 * nowhere on this page. Do not "tidy" them back into the groups.
 *
 * Provenance for every number below: the census in the closing
 * section, run 2026-09-21 across 13 accounts. Where a figure came off
 * a single tray database instead, the text says so on the spot. */
export default function LogCoveragePage() {
  return (
    <MarketingSurface
      navId="docs"
      crumb={[
        { label: 'Site', href: '/' },
        { label: 'Docs', href: '/docs' },
        { label: 'Log coverage' },
      ]}
      title="Log coverage"
      ctx="Docs · what gets read, and what does not"
    >
      <DocsIndex active="/docs/log-coverage" />
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
          What the logs say.
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
          Every figure on this site is read out of one file the game
          writes on your own machine. This is which lines are read, which
          ones the game stopped writing, and which ones have never
          appeared at all.
        </p>

        <section className="ss-about-section">
          <div className="ss-about-section-eyebrow">01 — The source</div>
          <h2>One file, read on your machine.</h2>
          <p>
            The desktop app reads <code>Game.log</code> — the file Star
            Citizen writes into its own install folder, one per channel
            (LIVE, PTU, EPTU and the rest). That is the only game file it
            reads. Not your saves, not your screenshots, not your network
            traffic.
          </p>
          <p>
            A line whose shape the parser recognises becomes an event. A
            line it does not recognise is kept on your machine as a
            candidate — the raw line, the five lines either side of it for
            context, and a flag on anything that looks like a handle, a
            shard id, a GEID or an IP address. Candidates are never
            uploaded on their own. You open one, see what it contains, and
            decide.
          </p>
          <p style={{ color: 'var(--fg-muted)' }}>
            Keeping the failures matters more than it sounds. CIG changes
            the log without announcing it, and every entry in the two
            sections below was found by noticing that something we used to
            read had gone quiet — never by being told.
          </p>
        </section>

        <section className="ss-about-section">
          <div className="ss-about-section-eyebrow">02 — What is read</div>
          <h2>Thirty-six kinds of line.</h2>
          <p>
            <strong>Session and shard.</strong> The game starting, your
            handle from the login response, the shard you joined, server
            transitions, the solar system being seeded, and the session
            ending. This is what makes a play session a session rather
            than a heap of timestamps.
          </p>
          <p>
            <strong>Where you are.</strong> Spawn resolution, terrain
            loads, and inventory requests at a location — three different
            lines that each name a place, stitched together into the one
            location you see on the topbar and in the travel timeline.
          </p>
          <p>
            <strong>Travel.</strong> Quantum target selected, the route,
            arrival, and the two contract-specific destination lines. This
            is where the journey map comes from.
          </p>
          <p>
            <strong>Survival.</strong> Your death, your incapacitation,
            and being ejected from a vehicle that has come apart around
            you. Only your own deaths reach the log this way, so a match
            is unambiguous.
          </p>
          <p>
            <strong>Contracts.</strong> Mission start, mission end with
            its outcome, and objective changes in between.
          </p>
          <p>
            <strong>Shops and commodities.</strong> Buy requests, the
            shop flow response — which says whether it went through, on
            the occasions the game bothers to say — and requests that
            timed out.
          </p>
          <p>
            <strong>Gear.</strong> Attachments received, equipment
            changes, vehicles stowed, and the burst summary that turns a
            spray of individual attachment lines into one loadout.
          </p>
          <p>
            <strong>Odds and ends.</strong> HUD notifications, and{' '}
            <code>remote_match</code> — the catch-all for a parser rule
            the server published after your copy of the app was built.
          </p>
          <p style={{ color: 'var(--fg-muted)' }}>
            Some of those are read but no longer arrive, because the game
            stopped writing them. Which ones is the next section, and it
            is the honest half of this list.
          </p>
        </section>

        <section className="ss-about-section">
          <div className="ss-about-section-eyebrow">03 — Retired</div>
          <h2>Lines that stopped arriving.</h2>
          <p>
            <strong>Kills, 19 November 2025.</strong>{' '}
            <code>&lt;Actor Death&gt;</code> was the line that named who
            killed whom. It accounted for 49,542 events across four
            accounts and then stopped, on that date, for everyone. That is
            why the NPC kill count shows nothing for almost anybody — not
            your play, and not a bug we introduced. The parser stays,
            because the rows it already produced are still real.
          </p>
          <p>
            <strong>Hull losses.</strong>{' '}
            <code>&lt;Vehicle Destruction&gt;</code> has never produced a
            single event, and there is no trace of it anywhere in the
            unparsed lines either. What survives is an ejection line that
            names the ship you were thrown out of and says why:{' '}
            <em>
              due to previous zone being in a destroyed vehicle with
              detached interior
            </em>
            . One tray database was carrying 371 of those, naming a
            Scythe, a Nox and a Tiburon, while the hull-loss figure sat at
            zero. It has been read since 20 September 2026.
          </p>
          <p style={{ color: 'var(--fg-muted)' }}>
            That ejection is deliberately <em>not</em> recorded as a
            death, even though the engine tags the line{' '}
            <code>[ActorState] Dead</code>. In a sample, only about 60% of
            them sat near a death already recorded from its own line —
            counting them would double up the majority and invent the
            rest. Losing the ship is the part the line proves, so that is
            the only part taken from it.
          </p>
          <p>
            <strong>Commodity buy and sell, September 2023.</strong> Trade
            figures dated before then are real. Nothing has been recorded
            since, on any account in the census.
          </p>
        </section>

        <section className="ss-about-section">
          <div className="ss-about-section-eyebrow">04 — The rule</div>
          <h2>A zero is a claim.</h2>
          <p>
            A retired line and a quiet week look identical from the
            outside: a confident 0 in a tile. A reader cannot tell them
            apart, so the site no longer asks them to. Where a figure is
            structurally unavailable the row is left out rather than
            drawn as zero, and a genuine zero — the line arriving, the
            count being none — still shows.
          </p>
          <p style={{ color: 'var(--fg-muted)' }}>
            The rule comes from being caught by it. Contracts started
            read zero on every account, beside 1,238 contracts ended,
            because the pattern expected <code>missionId[</code> and the
            game writes <code>missionId [</code> with a space. Ten
            thousand lines on a single machine went unread over one
            character, and the tile reported that as a fact about the
            player.
          </p>
          <p style={{ color: 'var(--fg-muted)' }}>
            The dates and counts above come from one census across 13
            accounts, run on 21 September 2026; the per-machine figures
            are called out as such where they appear, and they are one
            machine. They will drift as CIG changes the log. That is the
            nature of reading someone else&apos;s log file, and pretending
            otherwise is the thing this page exists not to do.
          </p>
        </section>

        <section className="ss-about-section">
          <div className="ss-about-section-eyebrow">Related</div>
          <h2>The rest of the honest version.</h2>
          <p>
            <Link href="/trust">/trust</Link> covers what leaves your
            machine and what StarStats sees about other players.{' '}
            <Link href={'/docs/rsi-cookie' as Route}>The RSI cookie</Link>{' '}
            is the other half of what the desktop app touches. Back to{' '}
            <Link href={'/docs' as Route}>the quickstart</Link>.
          </p>
        </section>
      </div>
    </MarketingSurface>
  );
}
