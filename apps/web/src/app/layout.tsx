import type { Metadata, Route } from 'next';
import Link from 'next/link';
import { cookies } from 'next/headers';
import { IBM_Plex_Sans, IBM_Plex_Mono, Michroma } from 'next/font/google';
import {
  getAppearanceConfig,
  getCurrentLocation,
  getPreferences,
  getSupporterStatus,
  getSummary,
  getLocationsVisited,
  listSharedWithMe,
  statusOf,
  type ResolvedLocation,
  type SupporterStatusDto,
} from '@/lib/api';
import { logger } from '@/lib/logger';
import { getCategoryBundle } from '@/lib/reference';
import { EMPTY_CATEGORY_BUNDLE, type ReferenceCatalog } from '@/lib/reference-types';
import { getSession } from '@/lib/session';
import { isBetaGateOn } from '@/lib/beta-gate';
import { getTheme } from '@/lib/theme';
import { QuantumWarpBackground } from '@/components/shell/QuantumWarpBackground';
import { TopBar } from '@/components/shell/TopBar';
import { LeftRail } from '@/components/shell/LeftRail';
import { TelemetryTicker } from '@/components/shell/TelemetryTicker';
import { DrawerScrim } from '@/components/shell/DrawerScrim';
import { MarketingNav } from '@/components/shell/MarketingNav';
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

  // Per-render shell data: current location for the TopBar chip
  // (carries its own entered_at anchor — see ResolvedLocation on the
  // server) and inbound-share count for the Sharing nav badge. Both
  // fail-soft to a "neutral" value (null / 0) so the shell never
  // crashes on a single API hiccup. Fetched in parallel to keep one
  // round trip. The trace fetch the chip *used* to need was dropped
  // when the server started returning entered_at directly — the old
  // trace-derived dwell silently capped at the trace window (24h),
  // making a >24h stay render as a frozen "here 23h 57m".
  let location: ResolvedLocation | null = null;
  let inboundShareCount = 0;
  // Caller's own supporter status — drives the TopBar chip so the
  // user sees their recognition on every page, not just /u/<handle>.
  // Fail-soft to null so a hiccup on /v1/me/supporter doesn't blank
  // the chrome.
  let supporter: SupporterStatusDto | null = null;
  // Pulled at the layout level so the TopBar's LocationChip can link
  // its location text to /kb/location/{slug}. The endpoint caches
  // server-side for 1h and is fetched (cached) by every signed-in
  // page anyway — pulling it here saves the per-page fetch in
  // aggregate. Failures degrade to plain-text chip rendering.
  let locationCatalog: ReferenceCatalog = EMPTY_CATEGORY_BUNDLE.catalog;
  // Headline lifetime figures streamed as bracketed frames in the
  // telemetry rail (bridge). Fetched here with the other shell data so
  // the rail is populated on every signed-in page; fail-soft to null.
  let eventsTotal: number | null = null;
  let locationsCount: number | null = null;
  if (session) {
    const [
      locResult,
      sharedResult,
      catalogResult,
      supporterResult,
      summaryResult,
      locationsResult,
    ] = await Promise.allSettled([
      getCurrentLocation(session.token),
      listSharedWithMe(session.token),
      getCategoryBundle('location'),
      getSupporterStatus(session.token),
      getSummary(session.token),
      getLocationsVisited(session.token),
    ]);
    if (locResult.status === 'fulfilled') {
      location = locResult.value;
    } else {
      logger.warn(
        {
          err: locResult.reason,
          call: 'topbar.location',
          status: statusOf(locResult.reason),
        },
        'topbar location fetch failed',
      );
    }
    if (sharedResult.status === 'fulfilled') {
      // Count active shares only — expired entries still appear in
      // the inbound list (recipients should know who used to share)
      // but the nav badge should reflect "things to look at now". An
      // expired badge would be noise and would never clear.
      const now = Date.now();
      inboundShareCount = sharedResult.value.shared_with_me.filter(
        (entry) =>
          !entry.expires_at ||
          new Date(entry.expires_at).getTime() > now,
      ).length;
    } else {
      logger.warn(
        {
          err: sharedResult.reason,
          call: 'topbar.shared',
          status: statusOf(sharedResult.reason),
        },
        'inbound share count fetch failed',
      );
    }
    if (catalogResult.status === 'fulfilled') {
      locationCatalog = catalogResult.value.catalog;
    } else {
      logger.warn(
        {
          err: catalogResult.reason,
          call: 'topbar.catalog',
          status: statusOf(catalogResult.reason),
        },
        'topbar location catalog fetch failed',
      );
    }
    if (supporterResult.status === 'fulfilled') {
      supporter = supporterResult.value;
    } else {
      logger.warn(
        {
          err: supporterResult.reason,
          call: 'topbar.supporter',
          status: statusOf(supporterResult.reason),
        },
        'topbar supporter status fetch failed',
      );
    }
    if (summaryResult.status === 'fulfilled') {
      eventsTotal = summaryResult.value.total;
    } else {
      logger.warn(
        {
          err: summaryResult.reason,
          call: 'rail.summary',
          status: statusOf(summaryResult.reason),
        },
        'rail summary fetch failed',
      );
    }
    if (locationsResult.status === 'fulfilled') {
      locationsCount = locationsResult.value.unique_locations;
    } else {
      logger.warn(
        {
          err: locationsResult.reason,
          call: 'rail.locations',
          status: statusOf(locationsResult.reason),
        },
        'rail locations fetch failed',
      );
    }
  }

  return (
    <html
      lang="en"
      data-theme={theme}
      data-wave-speed={waveSpeed}
      className={`${plexSans.variable} ${plexMono.variable} ${michroma.variable}`}
    >
      <body>
        {/*
          Skip-to-content link (M-W9): first focusable element in the
          DOM so keyboard/AT users can jump past the TopBar + LeftRail
          (or MarketingNav) straight to the page's `#main` wrapper.
          Visually hidden until focused — see `.ss-skip-link`.
        */}
        <a href="#main" className="ss-skip-link">
          Skip to content
        </a>
        <QuantumWarpBackground />
        {hasSession ? (
          <div
            className="ss-app"
            style={{ position: 'relative', zIndex: 1, minHeight: '100vh' }}
          >
            <TopBar
              handle={session.claimedHandle}
              location={location}
              dwellStart={location?.entered_at ?? null}
              dwellIsLowerBound={
                location?.entered_at_is_lower_bound ?? false
              }
              locationCatalog={locationCatalog}
              supporter={supporter}
              staffRoles={session.staffRoles}
              inboundShareCount={inboundShareCount}
            />
            <LeftRail
              handle={session.claimedHandle}
              staffRoles={session.staffRoles}
              inboundShareCount={inboundShareCount}
              location={location}
              supporter={supporter}
              eventsTotal={eventsTotal}
              locationsCount={locationsCount}
            />
            <DrawerScrim />
            <div className="ss-main" id="main" tabIndex={-1}>
              <TelemetryTicker
                location={location}
                supporter={supporter}
                eventsTotal={eventsTotal}
                locationsCount={locationsCount}
              />
              {!session.emailVerified && (
                <div className="unverified-banner" role="status">
                  <span>Email unverified — claim it before someone else can.</span>{' '}
                  <Link href="/settings#verification">Resend</Link>
                </div>
              )}
              {children}
            </div>
            {/*
              Audit v2 §07 polish: Lore moved out of the left rail into
              a calm footer — users hit it once after signup, so the
              rail entry was noise. Fine-print + privacy live here too.
              Brand book §11 compliance: About + Fankit + Fandom-FAQ
              outbound links plus the attribution chip are reachable
              from every signed-in surface.
            */}
            <footer
              style={{
                // `.ss-app` is a 2-col grid (220px rail | 1fr main). Without an
                // explicit grid-column, auto-flow drops the footer into row 3
                // column 1 (the 220px rail column) and it looks squished to
                // the left. Mirror `.ss-topbar { grid-column: 1 / -1 }` so
                // the footer spans the full width below rail + main.
                gridColumn: '1 / -1',
                textAlign: 'center',
                fontSize: 'var(--fs-xs)',
                color: 'var(--fg-dim)',
                padding: 'var(--s4) var(--s5)',
                display: 'flex',
                flexDirection: 'column',
                gap: 'var(--s3)',
                alignItems: 'center',
              }}
            >
              <div>
                <Link href={'/about' as Route} style={{ color: 'inherit' }}>
                  About
                </Link>
                <span aria-hidden="true"> · </span>
                <Link href={'/lore' as Route} style={{ color: 'inherit' }}>
                  Lore
                </Link>
                <span aria-hidden="true"> · </span>
                <Link href={'/changelog' as Route} style={{ color: 'inherit' }}>
                  Changelog
                </Link>
                <span aria-hidden="true"> · </span>
                <Link href={'/roadmap' as Route} style={{ color: 'inherit' }}>
                  Roadmap
                </Link>
                <span aria-hidden="true"> · </span>
                <a
                  // portal.starstats.app is the SSO front end to StarPlatform
                  // and the door a visitor should walk through.
                  // platform.starstats.app is the app behind it — linking
                  // there sends people past the sign-in they need.
                  href="https://portal.starstats.app"
                  target="_blank"
                  rel="noopener noreferrer"
                  style={{ color: 'inherit' }}
                  title="StarPlatform — the self-hosted companion for orgs, guilds, clans, teams and clubs"
                >
                  StarPlatform
                </a>
                <span aria-hidden="true"> · </span>
                <Link href={'/trust' as Route} style={{ color: 'inherit' }}>
                  Trust
                </Link>
                <span aria-hidden="true"> · </span>
                <Link href="/privacy" style={{ color: 'inherit' }}>
                  Privacy
                </Link>
                <span aria-hidden="true"> · </span>
                <Link href={'/terms' as Route} style={{ color: 'inherit' }}>
                  Terms
                </Link>
                <span aria-hidden="true"> · </span>
                <a
                  href="https://support.robertsspaceindustries.com/hc/en-us/articles/360006895793"
                  target="_blank"
                  rel="noopener noreferrer"
                  style={{ color: 'inherit' }}
                >
                  Fandom FAQ
                </a>
                <span aria-hidden="true"> · </span>
                <a
                  href="https://robertsspaceindustries.com/en/fankit"
                  target="_blank"
                  rel="noopener noreferrer"
                  style={{ color: 'inherit' }}
                >
                  RSI Fankit
                </a>
                <span aria-hidden="true"> · </span>
                <a
                  href="mailto:dojo@thecodesaiyan.io"
                  style={{ color: 'inherit' }}
                >
                  Contact
                </a>
              </div>
              <span className="ss-footer-attribution">
                Fan-made · Not affiliated with Cloud Imperium Games · RSI ·
                Star Citizen™ &amp; Squadron 42™ are trademarks of CIG ·
                Ship, vehicle, weapon &amp; item names and specifications ©
                Cloud Imperium Rights LLC / Cloud Imperium Rights Ltd —
                unofficial fan reference; facts only, see{' '}
                <Link
                  href="/about#community-data-sources"
                  style={{ color: 'inherit', textDecoration: 'underline' }}
                >
                  /about
                </Link>
                .
              </span>
              {/*
                Sub-footer line. Tiny, low-contrast version chip so
                operators can tell at a glance which platform build is
                serving the page without opening DevTools or
                /healthz. Per the release-tracks split spec, this is
                the *platform* version (server + web ship together);
                the tray has its own independent version stream.
                Build-time inlined via next.config.mjs's `env:` block
                from workspace Cargo.toml.
              */}
              <span
                style={{
                  marginTop: 4,
                  fontSize: 'var(--fs-2xs, 11px)',
                  opacity: 0.55,
                  letterSpacing: '0.02em',
                }}
              >
                platform v{process.env.NEXT_PUBLIC_PLATFORM_VERSION}
              </span>
            </footer>
          </div>
        ) : (
          <div
            style={{
              position: 'relative',
              zIndex: 1,
              minHeight: '100vh',
              display: 'flex',
              flexDirection: 'column',
            }}
          >
            <MarketingNav />
            <div id="main" tabIndex={-1} style={{ flex: 1 }}>
              {children}
            </div>
            {/*
              Marketing footer. Brand book §11 requires the verbatim
              fan-fiction disclaimer + outbound link block on every
              signed-out surface; the inner container caps at 1080px
              to match the marketing content column while the outer
              <footer> stays full-bleed so border-top spans the
              viewport.
            */}
            <footer
              className="site-footer"
              style={{
                padding: 'var(--s5) var(--s4)',
                fontSize: 'var(--fs-xs)',
                color: 'var(--fg-dim)',
                borderTop: '1px solid var(--border)',
              }}
            >
              <div
                className="site-footer-inner"
                style={{
                  display: 'flex',
                  flexDirection: 'column',
                  gap: 'var(--s3)',
                  alignItems: 'center',
                  maxWidth: 1080,
                  margin: '0 auto',
                }}
              >
              <div
                style={{
                  display: 'flex',
                  gap: 14,
                  flexWrap: 'wrap',
                  justifyContent: 'center',
                }}
              >
                <Link href={'/about' as Route}>About</Link>
                <span aria-hidden="true">·</span>
                <Link href="/features">Features</Link>
                <span aria-hidden="true">·</span>
                <Link href={'/star-platform' as Route}>StarPlatform</Link>
                <span aria-hidden="true">·</span>
                <Link href={'/lore' as Route}>Lore</Link>
                <span aria-hidden="true">·</span>
                <Link href={'/changelog' as Route}>Changelog</Link>
                <span aria-hidden="true">·</span>
                <Link href={'/roadmap' as Route}>Roadmap</Link>
                <span aria-hidden="true">·</span>
                <Link href={'/donate' as Route}>Donate</Link>
                <span aria-hidden="true">·</span>
                <Link href="/privacy">Privacy</Link>
                <span aria-hidden="true">·</span>
                <a
                  href="https://support.robertsspaceindustries.com/hc/en-us/articles/360006895793"
                  target="_blank"
                  rel="noopener noreferrer"
                >
                  Fandom FAQ
                </a>
                <span aria-hidden="true">·</span>
                <a
                  href="https://robertsspaceindustries.com/en/fankit"
                  target="_blank"
                  rel="noopener noreferrer"
                >
                  RSI Fankit
                </a>
                <span aria-hidden="true">·</span>
                <a href="mailto:dojo@thecodesaiyan.io">Contact</a>
              </div>
              <span
                className="ss-footer-attribution"
                style={{ textAlign: 'center', maxWidth: 640 }}
              >
                Fan-made · Not affiliated with Cloud Imperium Games · RSI ·
                Star Citizen™ &amp; Squadron 42™ are trademarks of CIG ·
                Ship, vehicle, weapon &amp; item names and specifications ©
                Cloud Imperium Rights LLC / Cloud Imperium Rights Ltd —
                unofficial fan reference; facts only, see{' '}
                <Link
                  href="/about#community-data-sources"
                  style={{ color: 'inherit', textDecoration: 'underline' }}
                >
                  /about
                </Link>
                .
              </span>
              {/*
                Sub-footer line, mirroring the signed-in footer above.
                See that block for rationale (platform version, build-
                time inlined, release-tracks split spec reference).
              */}
              <span
                style={{
                  marginTop: 4,
                  fontSize: 'var(--fs-2xs, 11px)',
                  opacity: 0.55,
                  letterSpacing: '0.02em',
                  textAlign: 'center',
                }}
              >
                platform v{process.env.NEXT_PUBLIC_PLATFORM_VERSION}
              </span>
              </div>
            </footer>
            {showBetaGate && <BetaGate />}
          </div>
        )}
      </body>
    </html>
  );
}
