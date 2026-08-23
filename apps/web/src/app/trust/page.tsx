import { MarketingSurface } from '@/components/projection/MarketingSurface';
import { LegalIndex } from '@/components/projection/LegalIndex';
import type { Metadata, Route } from 'next';
import Link from 'next/link';

export const metadata: Metadata = {
  title: 'Trust',
  description:
    'Can StarStats get you banned, what leaves your PC, what it sees about other players, and how to check every claim on this page yourself.',
};

const REPO = 'https://github.com/TheCodeSaiyan/StarStats-Platform';
const CONTACT_EMAIL = 'dojo@thecodesaiyan.io';

/* Reuses the /about section furniture (eyebrow + h2 plateau) so the two
 * read as one voice rather than two pages that happen to share a nav.
 * Ordering is deliberate and was argued over: EAC first because that is
 * the fear a suspicious player arrives with, then the bystander
 * disclosure immediately after — early, unprompted, and the thing no
 * marketing page would volunteer.
 *
 * §02 was FALSE until 2026-07-17 and shipped that way. It claimed we
 * store "whoever you fought, whoever was in your instance, whoever
 * killed you." None of that had a rule behind it: no parser rule reads
 * bystanders, party members, or an instance roster (every `Player[...]`
 * capture in parser.rs resolves to the LOCAL player), and CIG removed
 * the `<Actor Death>` line that carried `killer` (events.rs:191-195) —
 * which is why inference.rs:371-384 synthesizes PlayerDeath from
 * VehicleDestruction + ResolveSpawn at 0.85 confidence. `ActorDeath.killer`
 * (parser.rs:279) is real but only ever fires on legacy captures. It is
 * NOT CVar-gated; there is no verbosity that brings it back.
 *
 * The obvious correction — "we never upload other players' names" — is
 * ALSO false, which is why §02 now says two things. `raw_line` is the
 * verbatim line and it goes over the wire (wire.rs:24-27, sync.rs:1168),
 * with no allowlist on the drain (storage.rs:406) and no redaction on
 * that path at all. Rules bound what we READ, never what we STORE. So a
 * matched line that happens to name someone else is retained. Both halves
 * are true and the page has to say both. Do not "simplify" either away —
 * this is the same trap as §03, and this section has now fallen into it
 * once.
 *
 * §03 says "a paired tray uploads" on purpose. The tempting, wrong
 * simplification is "nothing leaves until you turn sync on" — this page
 * shipped that for one release and it was false. There are TWO flags:
 * `remote_sync.enabled` (set true by pair_device, commands.rs) drives the
 * tray's upload worker (sync.rs, `if !cfg.enabled`), while
 * `devices.sync_enabled` (the Cloud sync toggle, default FALSE) decides
 * whether the server ACCEPTS the batch (enforce_device_sync_gate in
 * ingest.rs). So a freshly-paired tray with sync off really does send
 * batches and really does get 403 device_sync_disabled — that is not
 * theory, it is why `is_sync_disabled_rejection` exists (a device that
 * treated the 403 as auth loss unpaired itself in a loop). Do not
 * "simplify" this back. Data leaves; the server refuses it. Both halves
 * are true and the page has to say both. */
export default function TrustPage() {
  return (
    <MarketingSurface
      navId="trust"
      crumb={[
        { label: 'Site', href: '/' },
        { label: 'Trust' },
      ]}
      title="Trust"
      ctx="What runs on your machine, and why it is safe"
    >
      <LegalIndex active="/trust" />
    <div className="ss-about">
      <div className="ss-placard" style={{ marginBottom: 'var(--s5)' }}>
        Trust
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
        No surprises. Check for yourself.
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
        You&apos;re about to give a third-party tool access to your Star
        Citizen play. You should be suspicious of that. This page is the
        honest version — including the parts that aren&apos;t flattering —
        and every claim on it links to something you can read yourself.
      </p>

      <section className="ss-about-section">
        <div className="ss-about-section-eyebrow">01 — The first question</div>
        <h2>It can&apos;t get you banned.</h2>
        <p>
          StarStats never touches the game. It reads a text file the game
          already writes to your own disk, and — only if you set it up — it
          reads your own RSI profile page using your own login. That&apos;s
          the whole surface.
        </p>
        <p>It does not, by design and not by policy:</p>
        <ul>
          <li>inject a DLL, overlay, or anything else into the game</li>
          <li>attach to the game process or read its memory</li>
          <li>sniff or modify game network traffic</li>
          <li>run macros or automate any in-game action</li>
        </ul>
        <p style={{ color: 'var(--fg-muted)' }}>
          Those are the things anti-cheat actually watches for, and they are
          the things a log reader has no reason to do. If you want the long
          version — including which Windows APIs it uses, in plain terms, and
          how it compares to tools that <em>do</em> get people banned — it&apos;s
          written up in{' '}
          <a href={`${REPO}/blob/main/EAC-SAFETY.md`}>EAC-SAFETY.md</a>.
        </p>
      </section>

      <section className="ss-about-section">
        <div className="ss-about-section-eyebrow">02 — The awkward part</div>
        <h2>We keep whole log lines, not just the bits we read.</h2>
        <p>
          Nobody volunteers this, so here it is second. StarStats never goes
          looking for other players. No rule in the parser reads them —
          there&apos;s nothing that records who was in your instance, who you
          fought, or who was flying next to you. And your killer&apos;s name
          isn&apos;t in a modern log to begin with: CIG stopped writing that
          line, which is why StarStats has to <em>infer</em> your death from
          your ship blowing up and you respawning.
        </p>
        <p>
          Here&apos;s the part that isn&apos;t flattering. When a line matches
          a rule, we upload the <em>whole line</em>, exactly as the game wrote
          it — not just the fields we parsed. We do that so we can re-read old
          events when the parser gets smarter, without asking you to upload
          anything again. It also means that if a log line ever happens to
          name someone else, that text is kept, whether or not anything reads
          it. We don&apos;t search it for handles. We also don&apos;t strip
          them out.
        </p>
        <p>
          So: no profile of anyone, no way to look up a player who never
          crossed your path, and nothing that treats another person as data.
          What there is, is verbatim retention — and we&apos;d rather write
          that down than let you find it out.
        </p>
        <p style={{ color: 'var(--fg-muted)' }}>
          The full detail, including how long it&apos;s kept and how to have
          it removed, is in{' '}
          <Link href="/privacy">the privacy policy</Link>, section 2.2. It is
          the section worth reading rather than skimming.
        </p>
      </section>

      <section className="ss-about-section">
        <div className="ss-about-section-eyebrow">03 — What leaves your PC</div>
        <h2>Nothing, until you pair it. Then two locks, both yours.</h2>
        <p>
          Install it and it is a local program. It parses your logs on your
          machine, keeps the result there, and has never spoken to this
          website about you. There is no account, no upload, nothing to
          opt out of.
        </p>
        <p>
          <strong>Pairing is the moment that changes.</strong> You generate a
          code on this site and type it into the tray — nothing pairs by
          accident. From then on the tray does start sending your parsed
          events. We&apos;d rather say that plainly than let you find out:
          a paired tray uploads.
        </p>
        <p>
          What it can&apos;t do is make them land. Your account holds a second
          lock — <em>Cloud sync</em>, off by default — and while it&apos;s off
          the server rejects every batch the tray sends and stores none of
          them. Two gates: one on your PC, one on your account, and the
          server-side one is the authority. Turning Cloud sync off later stops
          the data at the door even if the tray keeps knocking.
        </p>
        <p>
          What goes up, once both locks are open: the parsed events — logins,
          deaths, missions, jumps, locations, what you were flying. What never
          goes up: your chat, your inventory, your currency, your screen. Not
          as a promise — the parser has no code that can read those things.
        </p>
        <p style={{ color: 'var(--fg-muted)' }}>
          Who can see it once it&apos;s up is yours to set, per category, from{' '}
          <Link href="/sharing">your sharing settings</Link>. A public profile
          is a thing you choose, not a default you have to find and disable.
        </p>
      </section>

      <section className="ss-about-section">
        <div className="ss-about-section-eyebrow">04 — The cookie</div>
        <h2>The one that deserves a straight answer.</h2>
        <p>
          To read your RSI profile — your handle, your org, your hangar —
          StarStats asks you to paste your RSI session cookie. Let&apos;s be
          blunt about what that is: it&apos;s the thing that proves to the RSI
          website that you are you. Anything holding it can act as you on
          that site.
        </p>
        <p>
          So: it&apos;s stored in your operating system&apos;s credential
          store, not in a config file, and it never leaves your machine —
          StarStats uses it to talk to RSI directly from your PC, and the
          result is what syncs, not the cookie. You can revoke it any time by
          logging out of RSI in your browser, which invalidates the session.
        </p>
        <p style={{ color: 'var(--fg-muted)' }}>
          If that trade doesn&apos;t sit right with you, don&apos;t make it.
          StarStats works without the cookie; you just lose the RSI-side
          detail. That&apos;s a real option, not a dark pattern with a hidden
          skip link.
        </p>
      </section>

      <section className="ss-about-section">
        <div className="ss-about-section-eyebrow">05 — Leaving</div>
        <h2>You can take it all back.</h2>
        <p>
          Export your whole manifest whenever you like — the actual data, in
          NDJSON, ZIP or CSV, not a screenshot of a dashboard. Delete your
          account and it goes, on the schedule set out in{' '}
          <Link href="/privacy">the privacy policy</Link>.
        </p>
        <p style={{ color: 'var(--fg-muted)' }}>
          The client can re-parse your local logs from scratch, so leaving
          costs you nothing you can&apos;t rebuild. That&apos;s deliberate:
          the point is a record you own, and a record you can&apos;t take with
          you isn&apos;t one.
        </p>
      </section>

      <section className="ss-about-section">
        <div className="ss-about-section-eyebrow">06 — Verify it</div>
        <h2>Check my work.</h2>
        <p>
          Everything above is a claim. Here&apos;s how to stop taking my word
          for it:
        </p>
        <ul>
          <li>
            <a href={REPO}>The whole thing is public</a>, under MPL-2.0. The
            client, the server, the site. Read the parser and see for yourself
            what it can and can&apos;t touch.
          </li>
          <li>
            <a href={`${REPO}/blob/main/EAC-SAFETY.md`}>EAC-SAFETY.md</a> —
            the long answer to section 01, including the Windows APIs used.
          </li>
          <li>
            <a href={`${REPO}/blob/main/SECURITY.md`}>SECURITY.md</a> — how to
            report a vulnerability privately. It commits to acknowledging
            within 7 days and triaging within 30.
          </li>
          <li>
            <Link href="/privacy">The privacy policy</Link> names every
            sub-processor individually, and gives a retention period per item
            rather than a vague &ldquo;as long as necessary&rdquo;.
          </li>
          <li>
            <Link href={'/terms' as Route}>The terms</Link> say what happens
            to your data during the beta, in words rather than lawyer.
          </li>
        </ul>
        <p style={{ color: 'var(--fg-muted)' }}>
          One thing that number deserves context on: there is no security
          team. There is one person, and that 7-day clock is his, around a
          job and a life. It is a real commitment and it has been kept so
          far — the issue history is public, so you can check that too rather
          than believing it. If something looks wrong, email{' '}
          <a href={`mailto:${CONTACT_EMAIL}`}>{CONTACT_EMAIL}</a> and you will
          get a human, because there is only one.
        </p>
      </section>

      <p style={{ marginTop: 'var(--s6)' }}>
        <Link href="/">← Back to StarStats</Link>
      </p>
    </div>
    </MarketingSurface>
  );
}
