import Link from 'next/link';
import { redirect } from 'next/navigation';
import { getSession } from '@/lib/session';
import { HeroRotator } from './_components/HeroRotator';
import { HeroHeatmap } from './_components/HeroHeatmap';

const FEATURES: ReadonlyArray<{
  title: string;
  body: string;
  glyph: string;
}> = [
  {
    glyph: '◇',
    title: 'Stays on your PC by default',
    body:
      "Nothing leaves your machine until you sign in and turn on sync. You're in charge of when it talks to us.",
  },
  {
    glyph: '▦',
    title: 'Just what you did in-game',
    body:
      "Logins, deaths, missions, jumps. Never your chat, never other players, never anything the game doesn't already show you.",
  },
  {
    glyph: '⬡',
    title: 'A dashboard built like an instrument',
    body:
      'Your timeline rendered as cockpit instrumentation — a live activity heatmap, session log, travel and combat readouts, and a telemetry rail that streams your current stop. Four themes, one dense layout.',
  },
  {
    glyph: '◫',
    title: 'Knowledge base, built in',
    body:
      'Ship, weapon, item and location names come from a wiki-synced catalogue, so engine identifiers become real names. Hover any entity for stats; open its page for the full Ship Matrix sheet.',
  },
  {
    glyph: '▤',
    title: 'Your loadout, laid out',
    body:
      'The gear the game last restored, drawn on a body paperdoll — armour by slot, weapons and carried kit grouped and named, each linking straight to its knowledge-base page.',
  },
  {
    glyph: '⌖',
    title: 'Always know where you are',
    body:
      'Every session is tagged with where you were — system, planet, city, station — sorted into a proper location hierarchy, with a "you are here" readout on your current stop.',
  },
  {
    glyph: '↗',
    title: 'Share what you want, exactly',
    body:
      'Per-event visibility on top of profile-level controls: public, RSI org-only, named-handle grants with expiry, or fully private. Verify a handle is yours by pasting a code into your bio for a minute.',
  },
  {
    glyph: '◈',
    title: 'Orgs, and StarPlatform',
    body:
      'RSI org owners get a shared dashboard with roles enforced by Zanzibar-style ReBAC. Run a whole group — org, guild, clan or crew — on the self-hosted StarPlatform companion.',
  },
  {
    glyph: '✦',
    title: 'Every PC you play on, one timeline',
    body:
      'Pair as many machines as you like — it all lands in one timeline, no double-counts. Your theme and preferences sync across them, opt-in per device, revocable from the web.',
  },
  {
    glyph: '◉',
    title: 'Records that find themselves',
    body:
      'Your deadliest single session, busiest week, longest streak — surfaced from the same timeline so you can drill into where they came from.',
  },
  {
    glyph: '⤓',
    title: 'Your numbers, your file',
    body:
      'Per-day heatmap, top activities, full timeline. Download the whole manifest as a single file whenever you want.',
  },
  {
    glyph: '⊞',
    title: 'Locked-down sign-in',
    body:
      'Magic link or password, two-factor with backup codes, per-device pairing. Handle verification stops anyone claiming yours.',
  },
];

export default async function HomePage() {
  const session = await getSession();
  if (session) redirect('/me');

  return (
    <div className="ss-landing" style={{ minHeight: '100%', position: 'relative' }}>
      <main
        style={{
          position: 'relative',
          zIndex: 1,
          maxWidth: 'none',
          margin: 0,
          padding: 0,
        }}
      >
        {/* Hero */}
        <section
          className="ss-hero"
          style={{
            padding: '120px 48px 80px',
            maxWidth: 1280,
            margin: '0 auto',
            display: 'grid',
            gridTemplateColumns: '1.2fr 1fr',
            gap: 64,
          }}
        >
          <div>
            {/*
              Per the design audit v2 (§07 — landing verdict): patch
              announcements belong in release notes, not the hero.
              The eyebrow that read "v0.0.X-beta · Pyro patch ready"
              was removed for that reason — the h1 is the anchor.
            */}
            <h1
              style={{
                margin: 0,
                fontWeight: 600,
                fontSize: 'clamp(40px, 6vw, 76px)',
                lineHeight: 1.02,
                letterSpacing: '-0.025em',
                color: 'var(--fg)',
              }}
            >
              <span
                style={{
                  color: 'var(--fg-muted)',
                  fontSize: '0.6em',
                  display: 'block',
                  marginBottom: 12,
                  fontWeight: 400,
                  letterSpacing: '-0.01em',
                }}
              >
                Track your Star Citizen play.
              </span>
              <HeroRotator />
            </h1>
            <p
              style={{
                color: 'var(--fg-muted)',
                fontSize: 'var(--fs-lg)',
                lineHeight: 1.55,
                maxWidth: 560,
                marginTop: 28,
              }}
            >
              A small app on your PC reads what the game already writes
              down — when you log in, where you fly, what you fly with. It
              stays on your machine until you sign in and turn on sync.
              Never your chat. Never anyone else.
            </p>
            <div
              className="ss-hero-buttons"
              style={{ display: 'flex', gap: 12, marginTop: 36 }}
            >
              <Link href="/auth/signup" className="ss-btn ss-btn--primary">
                Create account →
              </Link>
              <Link href="/downloads" className="ss-btn ss-btn--ghost">
                Download tray client
              </Link>
            </div>
            <div
              style={{
                marginTop: 32,
                display: 'flex',
                gap: 24,
                alignItems: 'center',
                color: 'var(--fg-dim)',
                fontSize: 12,
              }}
            >
              <span style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                ✓ Local-first
              </span>
              <span style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                ✓ Open source client
              </span>
              <span style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                ✓ Not affiliated with CIG
              </span>
            </div>
          </div>

          {/* Hero mockup — stylised stat card preview */}
          <div
            className="ss-hero-mock"
            style={{
              position: 'relative',
              display: 'flex',
              alignItems: 'center',
              justifyContent: 'center',
            }}
          >
            <div
              style={{
                position: 'absolute',
                inset: -40,
                background:
                  'radial-gradient(circle at 60% 40%, var(--accent-glow), transparent 60%)',
                filter: 'blur(20px)',
                opacity: 0.5,
                pointerEvents: 'none',
              }}
            />
            <div
              className="ss-card ss-card--elev"
              style={{
                position: 'relative',
                width: '100%',
                maxWidth: 460,
                padding: '22px 24px',
                transform: 'rotate(-1deg)',
              }}
            >
              <div
                style={{
                  display: 'flex',
                  alignItems: 'center',
                  justifyContent: 'space-between',
                  marginBottom: 18,
                }}
              >
                <div className="ss-placard">Last 26 weeks</div>
                <span className="ss-badge ss-badge--accent">
                  <span className="ss-badge-dot" />
                  Live
                </span>
              </div>
              <HeroHeatmap />
              <hr className="ss-rule" style={{ margin: '18px 0' }} />
              <div
                style={{
                  display: 'flex',
                  gap: 20,
                  alignItems: 'center',
                  flexWrap: 'nowrap',
                }}
              >
                <div style={{ display: 'flex', flexDirection: 'column', gap: 2 }}>
                  <div className="ss-placard">Sessions</div>
                  <div
                    className="mono"
                    style={{
                      fontSize: 'var(--fs-lg)',
                      color: 'var(--fg)',
                      letterSpacing: '-0.01em',
                    }}
                  >
                    184
                  </div>
                </div>
                <div style={{ display: 'flex', flexDirection: 'column', gap: 2 }}>
                  <div className="ss-placard">Hours</div>
                  <div
                    className="mono"
                    style={{
                      fontSize: 'var(--fs-lg)',
                      color: 'var(--fg)',
                      letterSpacing: '-0.01em',
                    }}
                  >
                    312.7
                  </div>
                </div>
                <div style={{ display: 'flex', flexDirection: 'column', gap: 2 }}>
                  <div className="ss-placard">Top type</div>
                  <div
                    className="mono"
                    style={{ fontSize: 13, color: 'var(--accent)' }}
                  >
                    quantum_target
                  </div>
                </div>
              </div>
            </div>
          </div>
        </section>

        {/* Features grid */}
        <section
          id="features"
          className="ss-features"
          style={{
            maxWidth: 1280,
            margin: '0 auto',
            padding: '0 48px 100px',
          }}
        >
          <div className="ss-placard" style={{ marginBottom: 16 }}>
            What you get
          </div>
          <h2
            style={{
              margin: '0 0 var(--s7)',
              fontSize: 'clamp(28px, 4vw, 40px)',
              fontWeight: 600,
              letterSpacing: 'var(--tracking-tight)',
              color: 'var(--fg)',
              maxWidth: 720,
            }}
          >
            A telemetry tool, not a fan shrine. Built for players who want
            to read their own footprint.
          </h2>

          <div
            data-rspgrid="3"
            style={{
              display: 'grid',
              gridTemplateColumns: 'repeat(3, 1fr)',
              gap: 'var(--s4)',
            }}
          >
            {FEATURES.map((f) => (
              <div key={f.title} className="ss-card" style={{ padding: 'var(--s5)' }}>
                <div
                  style={{
                    width: 32,
                    height: 32,
                    borderRadius: 'var(--r-xl)',
                    background: 'var(--accent-soft)',
                    color: 'var(--accent)',
                    display: 'grid',
                    placeItems: 'center',
                    marginBottom: 14,
                    border:
                      '1px solid color-mix(in oklab, var(--accent) 30%, transparent)',
                    fontSize: 16,
                    fontFamily: 'var(--font-mono)',
                  }}
                  aria-hidden
                >
                  {f.glyph}
                </div>
                <h3
                  style={{
                    margin: '0 0 var(--s2)',
                    fontSize: 'var(--fs-base)',
                    fontWeight: 600,
                    letterSpacing: '-0.01em',
                  }}
                >
                  {f.title}
                </h3>
                <p
                  style={{
                    margin: 0,
                    color: 'var(--fg-muted)',
                    fontSize: 'var(--fs-sm)',
                    lineHeight: 1.55,
                  }}
                >
                  {f.body}
                </p>
              </div>
            ))}
          </div>
        </section>
      </main>
    </div>
  );
}
