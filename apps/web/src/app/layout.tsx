import type { Metadata } from 'next';
import Link from 'next/link';
import { cookies } from 'next/headers';
import {
  IBM_Plex_Sans,
  IBM_Plex_Mono,
  Michroma,
  Chakra_Petch,
} from 'next/font/google';
import {
  getAppearanceConfig,
  getPreferences,
  listSharedWithMe,
} from '@/lib/api';
import { logger } from '@/lib/logger';
import { getSession } from '@/lib/session';
import { isBetaGateOn } from '@/lib/beta-gate';
import { isNoindexDeployment } from '@/lib/deployment';
import { getTheme } from '@/lib/theme';
import { ShellDataProvider } from '@/components/projection/ShellData';
import { BetaGate } from '@/app/_components/BetaGate';
import './globals.css';

// Phase-1 "bridge" type system. IBM Plex Sans = body; IBM Plex Mono = all
// figures (tabular numerals); Michroma = placards + eyebrows ONLY. Loaded
// via next/font/google, which self-hosts the font files at build time —
// no runtime request to Google (CSP-safe). Each exposes a CSS variable
// consumed by --font-sans / --font-mono / --font-display in
// starstats-tokens.css, so existing var(--font-*) consumers pick these up
// with no component change.
const plexSans = IBM_Plex_Sans({
  subsets: ['latin'],
  weight: ['400', '500', '600', '700'],
  variable: '--font-ibm-plex-sans',
  display: 'swap',
});
const plexMono = IBM_Plex_Mono({
  subsets: ['latin'],
  weight: ['400', '500', '600'],
  variable: '--font-ibm-plex-mono',
  display: 'swap',
});
const michroma = Michroma({
  subsets: ['latin'],
  weight: '400',
  variable: '--font-michroma',
  display: 'swap',
});

// The projection's ONE face. The design system loads it from the Google CDN via
// an `@import`; here it goes through next/font/google like the other three, so
// it is self-hosted at build time — no render-blocking third-party request and
// no extra font origin to allow in the CSP. `tokens-typography.css` names the
// family directly, and this variable is what backs it.
//
// The three flat families stay until the port completes. They go the day the
// last flat page does.
const chakra = Chakra_Petch({
  subsets: ['latin'],
  weight: ['300', '400', '500', '600', '700'],
  variable: '--font-chakra-petch',
  display: 'swap',
});

// metadataBase lets relative OG/Twitter image paths resolve in production.
// STARSTATS_SITE_URL is the deployment's canonical origin.
//
// Deliberately NOT prefixed NEXT_PUBLIC_. metadataBase is only ever read
// server-side while composing metadata, so it has no reason to reach the
// browser — and the prefix would actively break configuring it: Next
// inlines every NEXT_PUBLIC_* at build time (webpack DefinePlugin, server
// components included), so under `output: 'standalone'` a compose-set
// value would be read by nothing and a restart would change nothing.
// Unprefixed, the standalone server reads it from the environment at
// runtime — same mechanism as STARSTATS_API_URL. See
// home-servers-build:compose/starstats/compose.yml.
//
// The fallback MUST be a host that actually resolves. It previously read
// starstats.dev — the domain named in the brand handoff, which the project
// never shipped on and which does not resolve at all. Nothing failed
// locally or in CI: the pages render, the tags are well formed, and
// public/social/og.png exists — but every link unfurler (Reddit, Discord,
// Twitter, Slack) resolved og:image against the dead host, so every share
// rendered a broken preview. The break is only ever visible off-site.
const siteUrl = process.env.STARSTATS_SITE_URL ?? 'https://starstats.app';

export const metadata: Metadata = {
  metadataBase: new URL(siteUrl),
  // Staging (beta.starstats.app, STARSTATS_NOINDEX=1) must not be
  // indexed: it serves the same routes as production against the same
  // live data, so an indexed copy is duplicate content pointing at an
  // unfinished UI. Read at module load, which under `output:
  // 'standalone'` is server start — i.e. runtime, not build time, so
  // one image serves both environments. `/robots.txt` carries the
  // matching Disallow (see `app/robots.ts`); this tag covers URLs a
  // crawler reaches without asking robots.txt first.
  ...(isNoindexDeployment()
    ? { robots: { index: false, follow: false, nocache: true } }
    : {}),
  title: {
    default: 'StarStats',
    template: '%s — StarStats',
  },
  description:
    'A fan-built, local-first event tracker and manifest archive for Star Citizen players. Not affiliated with Cloud Imperium Games or Roberts Space Industries.',
  openGraph: {
    type: 'website',
    siteName: 'StarStats',
    title: 'StarStats — A pilot’s logbook.',
    description:
      'Fan-built, local-first Star Citizen event tracker. Pairs with the game, parses your logs on your machine, exports anywhere.',
    images: [{ url: '/social/og.png', width: 1200, height: 630, alt: 'StarStats' }],
  },
  twitter: {
    card: 'summary_large_image',
    title: 'StarStats — A pilot’s logbook.',
    description:
      'Fan-built, local-first Star Citizen event tracker. Pairs with the game, parses your logs on your machine, exports anywhere.',
    images: ['/social/og.png'],
  },
  // Favicon + Apple touch icon are file-system based — see
  // `app/icon.png` and `app/apple-icon.png`. Next.js auto-generates
  // <link rel="icon"> + <link rel="apple-touch-icon"> with the right
  // sizes from those files; no explicit metadata.icons needed.
};

export default async function RootLayout({
  children,
}: Readonly<{ children: React.ReactNode }>) {
  const session = await getSession();
  const theme = await getTheme(session?.token);
  const hasSession = session !== null;

  // Theme-switch wave speed: per-user preference wins when set, else the
  // sitewide `appearance_config` default, else 'normal'. The sitewide
  // read is unauthenticated so signed-out visitors get a real value too
  // (not just the client-side DEFAULT_DURATION fallback in
  // theme-transition.ts). Fail-soft on both reads — a hiccup here must
  // never block the shell from rendering.
  let waveSpeed = 'normal';
  try {
    const appearance = await getAppearanceConfig();
    if (appearance.theme_wave_speed) waveSpeed = appearance.theme_wave_speed;
  } catch (e) {
    logger.warn(
      { err: e, call: 'shell.appearance' },
      'appearance config fetch failed; defaulting wave speed to normal',
    );
  }
  if (session) {
    try {
      const prefs = await getPreferences(session.token);
      if (prefs.theme_wave_speed) waveSpeed = prefs.theme_wave_speed;
    } catch (e) {
      logger.warn(
        { err: e, call: 'shell.wave_speed_pref' },
        'preferences fetch failed while resolving wave speed',
      );
    }
  }

  // Beta overlay: shown only when the server gate is on AND the visitor
  // hasn't dismissed it. Signed-in users never see it (they're redirected
  // to /me from the landing anyway). Fail CLOSED — a status blip must never
  // trap visitors behind an overlay, and server-side signup enforcement is
  // the authoritative gate regardless.
  let showBetaGate = false;
  if (!session) {
    const dismissed =
      (await cookies()).get('ss_beta_dismissed')?.value === '1';
    // Shared with the auth-page banners via `isBetaGateOn` so the three
    // surfaces cannot drift into disagreeing about whether the beta is
    // on — or into one of them failing OPEN while the others fail
    // closed. The helper owns the fail-closed posture.
    if (!dismissed) {
      showBetaGate = await isBetaGateOn();
    }
  }

  /**
   * The one piece of shell data still fetched here.
   *
   * This block used to pull FIVE things for the flat chrome: the current
   * location and its catalog for the TopBar chip, the caller's supporter
   * status, and lifetime event/location totals for the telemetry rail. That
   * chrome is gone — every surface is a projection with its own `ChromeBar`,
   * and `/me` fetches its own readouts — so those four fetches were work done
   * on every signed-in page for markup nobody could see.
   *
   * The inbound-share count stays, because the badge it feeds is genuinely
   * global: it is the one notification the product has, and it belongs on every
   * page rather than only where someone remembered to fetch it.
   *
   * `allSettled` for one call is overkill, so this is a plain guarded fetch —
   * but the fail-soft is the same: a hiccup here must never take a page down.
   */
  let inboundShareCount = 0;
  if (session) {
    try {
      const shared = await listSharedWithMe(session.token);
      // EXPIRY, not revocation: an expired share stays in the inbound list
      // (recipients should know who used to share) but the badge should reflect
      // things to look at NOW. An expired badge would be noise and would never
      // clear.
      const now = Date.now();
      inboundShareCount = shared.shared_with_me.filter(
        (entry) =>
          !entry.expires_at || new Date(entry.expires_at).getTime() > now,
      ).length;
    } catch (e) {
      logger.warn({ err: e, call: 'shell.sharedWithMe' }, 'inbound share count fetch failed');
    }
  }

  return (
    <html
      lang="en"
      data-theme={theme}
      data-wave-speed={waveSpeed}
      className={`${plexSans.variable} ${plexMono.variable} ${michroma.variable} ${chakra.variable}`}
    >
      <body>
        {/*
          NO SKIP LINK HERE ANY MORE.

          This used to be the first focusable element, jumping past the flat
          `TopBar` and `LeftRail` to `#main`. Both are gone, and `Projection`
          renders its own skip link targeting `#hp-content` — which is the
          landmark now. Keeping this one meant TWO "Skip to content" links on
          every page, the first of them pointing at a wrapper that no longer
          contains the chrome it was there to skip.
        */}
        {/* Chrome-level facts every projection surface needs. The layout is
            the one place that still wraps every route and already had the
            inbound-share count, so the badge is fed from here rather than
            threaded through a dozen shells. */}
        <ShellDataProvider inboundShares={inboundShareCount}>
        {hasSession ? (
          <div className="ss-app" style={{ position: 'relative', zIndex: 1, minHeight: '100vh' }}>
            <div className="ss-main" id="main" tabIndex={-1}>
              {!session.emailVerified && (
                <div className="unverified-banner" role="status">
                  <span>Email unverified — claim it before someone else can.</span>{' '}
                  <Link href="/settings#verification">Resend</Link>
                </div>
              )}
              {children}
            </div>
          </div>
        ) : (
          <div style={{ position: 'relative', zIndex: 1, minHeight: '100vh' }}>
            <div id="main" tabIndex={-1}>
              {children}
            </div>
          </div>
        )}
        {showBetaGate && <BetaGate />}
        </ShellDataProvider>
      </body>
    </html>
  );
}
