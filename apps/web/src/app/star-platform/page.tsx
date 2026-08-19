import type { Metadata } from 'next';
import { redirect } from 'next/navigation';

import { getSession } from '@/lib/session';

export const metadata: Metadata = {
  // No brand suffix here: layout.tsx's title.template appends " — StarStats".
  // openGraph.title below keeps it, since a social card stands alone.
  title: 'StarPlatform',
  description:
    'StarPlatform — a self-hosted companion for any Star Citizen group: orgs, guilds, clans, teams, and clubs. Live roster, fleet movement, and an ops board, fed by members’ StarStats trays over an opt-in, read-only presence link.',
  openGraph: {
    title: 'StarPlatform · StarStats',
    description:
      'A self-hosted companion for Star Citizen orgs, guilds, clans, teams, and clubs: live presence, roster, fleet, and ops — fed by your members’ StarStats trays.',
    images: [{ url: '/social/og.png', width: 1200, height: 630 }],
  },
};

// The SSO front end to StarPlatform — the door a visitor should walk
// through. NOT platform.starstats.app: that is the app behind the portal,
// and linking there sends people past the sign-in they need. (A third
// host, starplatform.starstats.app, serves the same image as platform.)
const PLATFORM_URL = 'https://portal.starstats.app';

// StarPlatform manages player groups of every shape — we lead with the
// breadth rather than the single in-game word "org" so guilds, clans,
// teams, and clubs see themselves in it too. Neutral group words only;
// no overloaded in-game nouns (Fleet/Hangar) as a group label.
const GROUP_TYPES = ['Orgs', 'Guilds', 'Clans', 'Teams', 'Clubs', 'Crews'];

const POINTS: Array<{ title: string; body: string }> = [
  {
    title: 'Live presence',
    body: 'See who’s online and roughly where, in real time — derived from the same Game.log StarStats already reads. Zone and state only; never coordinates.',
  },
  {
    title: 'Roster & fleet',
    body: 'A permission-scoped roster and signed-out vehicle tracking, so leads see the whole-group picture and members see their own slice.',
  },
  {
    title: 'Ops board',
    body: 'Pin a live operation and a member’s HUD surfaces the orders that matter while they’re flying it.',
  },
];

/**
 * Marketing page for StarPlatform — the StarStats group companion (entered
 * via portal.starstats.app). Short by design: hero + three points + a CTA
 * out to the platform. Mirrors the `/features` / `/about` marketing
 * convention (placard + clamped h1 + lede + ss-card grid); the signed-out
 * nav + footer come from `app/layout.tsx`, so this renders content only.
 * Signed-in users are redirected into the app shell. (Renamed from the
 * former "Org platform"; `/org-platform` now redirects here.)
 */
export default async function StarPlatformPage() {
  const session = await getSession();
  if (session) redirect('/me');

  return (
    <div className="ss-landing" style={{ minHeight: '100%', position: 'relative' }}>
      <main
        style={{
          position: 'relative',
          zIndex: 1,
          maxWidth: 1080,
          margin: '0 auto',
          padding: '96px 48px 72px',
        }}
      >
        <span
          className="ss-placard"
          style={{ color: 'var(--fg-dim)', display: 'inline-block', marginBottom: 'var(--s3)' }}
        >
          StarPlatform
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
          Command your org, live.
        </h1>
        <p
          style={{
            margin: 'var(--s5) 0 0',
            maxWidth: '62ch',
            color: 'var(--fg-muted)',
            fontSize: 'var(--fs-lg)',
            lineHeight: 1.6,
          }}
        >
          Org, guild, clan, team, club — whatever your group calls itself,
          StarPlatform keeps it running. It’s a self-hosted companion for Star
          Citizen groups: a live roster, fleet movement, and an ops board, fed
          by your members’ StarStats trays over an{' '}
          <strong style={{ color: 'var(--fg)' }}>opt-in, read-only</strong>{' '}
          presence link. It rides the same Game.log boundary StarStats already
          crosses: nothing new is read, and no coordinates ever leave the member.
        </p>

        <p
          style={{
            margin: 'var(--s4) 0 0',
            maxWidth: '62ch',
            color: 'var(--fg-dim)',
            fontSize: 'var(--fs-base)',
            lineHeight: 1.6,
          }}
        >
          Born from wanting to automate our own Star Citizen org — and grown to
          fit any group that runs together.
        </p>

        <ul
          aria-label="Group types StarPlatform manages"
          style={{
            display: 'flex',
            flexWrap: 'wrap',
            gap: 'var(--s2)',
            listStyle: 'none',
            margin: 'var(--s5) 0 0',
            padding: 0,
          }}
        >
          {GROUP_TYPES.map((g) => (
            <li
              key={g}
              style={{
                border: '1px solid var(--border)',
                borderRadius: 'var(--r-pill)',
                padding: 'var(--s1) var(--s3)',
                fontSize: 'var(--fs-sm)',
                color: 'var(--fg-muted)',
              }}
            >
              {g}
            </li>
          ))}
        </ul>

        <div style={{ display: 'flex', gap: 'var(--s3)', flexWrap: 'wrap', marginTop: 'var(--s6)' }}>
          <a
            href={PLATFORM_URL}
            target="_blank"
            rel="noopener noreferrer"
            className="ss-btn ss-btn--primary"
          >
            Visit StarPlatform →
          </a>
        </div>

        <div
          style={{
            display: 'grid',
            gridTemplateColumns: 'repeat(auto-fit, minmax(240px, 1fr))',
            gap: 'var(--s4)',
            marginTop: 'var(--s7)',
          }}
        >
          {POINTS.map((p) => (
            <section key={p.title} className="ss-card" style={{ padding: 'var(--s4) var(--s5)' }}>
              <h2 style={{ margin: 0, fontSize: 'var(--fs-md)', fontWeight: 600 }}>{p.title}</h2>
              <p
                style={{
                  margin: 'var(--s2) 0 0',
                  color: 'var(--fg-muted)',
                  fontSize: 'var(--fs-base)',
                  lineHeight: 1.6,
                }}
              >
                {p.body}
              </p>
            </section>
          ))}
        </div>

        <p style={{ marginTop: 'var(--s6)', color: 'var(--fg-dim)', fontSize: 'var(--fs-sm)', lineHeight: 1.6, maxWidth: '62ch' }}>
          Self-hosted — you run it. Opt-in on both ends: members enable the
          connector in their tray (Settings → StarPlatform connector) and link
          on the platform. Either side can turn it off at any time.
        </p>
      </main>
    </div>
  );
}
