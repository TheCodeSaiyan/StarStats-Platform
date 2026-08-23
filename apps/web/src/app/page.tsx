/**
 * `/` — the landing page.
 *
 * GROUNDED PORT. `HoloLanding.jsx` is one of only three web screens COVERAGE
 * marks as read from real source, and it was read from THIS route — so the
 * projection shape is followed closely here in a way it deliberately is not
 * anywhere else in this port.
 *
 * The volume sells; the reading half below it explains. Both halves carry the
 * same claims, because `CalloutField` caps at six and hides itself entirely
 * below 1180px — a phone visitor must lose decoration, never information.
 *
 * NO INVENTED FIGURES. The kit's callouts read "92,481 events read" and
 * "Six panes"; a landing page has no reader to draw a figure from, so every
 * value below is sourced — the live version from the same GitHub release feed
 * the Emitter page reads, the rest from the repository itself. When the feed is
 * unreachable the version callout is dropped rather than guessed.
 */
import type { Metadata } from 'next';
import Link from 'next/link';
import type { Route } from 'next';
import { redirect } from 'next/navigation';
import { Pane, Plane, SubStats, MeterRow, BeamChip, type Calibration } from 'holo';
import { SiteLegalPlate } from '@/components/projection/SiteLegalPlate';
import { getSession } from '@/lib/session';
import { getTheme } from '@/lib/theme';
import { navSections } from '@/lib/nav';
import { setCalibrationAction } from '@/app/me/_projection/actions';
import { fetchTrayReleases } from '@/lib/github-releases.server';
import {
  LandingProjection,
  type LandingCallout,
} from './_projection/LandingProjection';

export const metadata: Metadata = {
  title: 'StarStats',
};

/**
 * The hero rotation, carried over verbatim from the flat hero this replaces.
 *
 * NOT the kit's `['Deaths.', 'Jumps.', 'Contracts.', 'Sessions.']`, whose
 * prompt claims they match the product's hero. They do not: the shipped hero
 * rotates a possessive claim, not a list of event types.
 *
 * The flat `HeroRotator` and `HeroHeatmap` islands were DELETED with this port
 * rather than left behind — nothing imported them once the volume replaced the
 * two-column hero, and a dead module that still looks live is how the tray's
 * In-Transit filter went a release doing nothing. `BrandHero` carries
 * `HeroRotator`'s one load-bearing behaviour forward: pinning the rotation
 * under `prefers-reduced-motion`, because the content swap itself registers as
 * motion even once the CSS sweep is flattened.
 */
const HERO_WORDS = [
  'StarStats.',
  'Your manifest.',
  'Your numbers.',
  'Your timeline.',
] as const;

/**
 * "What StarStats does NOT do", verbatim from the repository README.
 *
 * This is the product's sharpest claim and the reason it can be trusted next to
 * an anti-cheat, so it is quoted rather than summarised. The README's own
 * closing line: "If a feature would require any of the above, it doesn't belong
 * in StarStats."
 */
const EXCLUSIONS = [
  'Read game memory.',
  'Inject into the game process.',
  'Hook game APIs.',
  'Sniff or modify game network traffic.',
  'Modify any game files.',
  'Drive in-game input (no macros, no aimbots, no multiboxing automation).',
  "Touch other players' data — only your own log file and your own RSI session.",
] as const;

const FEATURES: ReadonlyArray<{ title: string; body: string }> = [
  {
    title: 'Stays on your PC by default',
    body:
      "Nothing leaves your machine until you sign in and turn on sync. You're in charge of when it talks to us.",
  },
  {
    title: 'Just what you did in-game',
    body:
      "Logins, deaths, missions, jumps. Never your chat, never other players, never anything the game doesn't already show you.",
  },
  {
    title: 'A dashboard built like an instrument',
    body:
      'Your timeline rendered as cockpit instrumentation — a live activity heatmap, session log, travel and combat readouts, and a telemetry rail that streams your current stop. Four themes, one dense layout.',
  },
  {
    title: 'Knowledge base, built in',
    body:
      'Ship, weapon, item and location names come from a wiki-synced catalogue, so engine identifiers become real names. Hover any entity for stats; open its page for the full Ship Matrix sheet.',
  },
  {
    title: 'Your loadout, laid out',
    body:
      'The gear the game last restored, drawn on a body paperdoll — armour by slot, weapons and carried kit grouped and named, each linking straight to its knowledge-base page.',
  },
  {
    title: 'Always know where you are',
    body:
      'Every session is tagged with where you were — system, planet, city, station — sorted into a proper location hierarchy, with a "you are here" readout on your current stop.',
  },
  {
    title: 'Share what you want, exactly',
    body:
      'Per-event visibility on top of profile-level controls: public, RSI org-only, named-handle grants with expiry, or fully private. Verify a handle is yours by pasting a code into your bio for a minute.',
  },
  {
    title: 'Orgs, and StarPlatform',
    body:
      'RSI org owners get a shared dashboard with roles enforced by Zanzibar-style ReBAC. Run a whole group — org, guild, clan or crew — on the self-hosted StarPlatform companion.',
  },
  {
    title: 'Every PC you play on, one timeline',
    body:
      'Pair as many machines as you like — it all lands in one timeline, no double-counts. Your theme and preferences sync across them, opt-in per device, revocable from the web.',
  },
  {
    title: 'Records that find themselves',
    body:
      'Your deadliest single session, busiest week, longest streak — surfaced from the same timeline so you can drill into where they came from.',
  },
  {
    title: 'Your numbers, your file',
    body:
      'Per-day heatmap, top activities, full timeline. Download the whole manifest as a single file whenever you want.',
  },
  {
    title: 'Locked-down sign-in',
    body:
      'Magic link or password, two-factor with backup codes, per-device pairing. Handle verification stops anyone claiming yours.',
  },
];


export default async function HomePage() {
  const session = await getSession();
  if (session) redirect('/me');

  let calibration: Calibration = 'terra';
  try {
    calibration = (await getTheme(undefined)) as Calibration;
  } catch {
    // Preference read failed; the default stands.
  }

  // The live channel version, from the same feed `/downloads` reads. A GitHub
  // outage drops the callout instead of printing a stale or invented number.
  let version: string | null = null;
  try {
    version = (await fetchTrayReleases()).stable?.version ?? null;
  } catch (err) {
    console.error('tray releases fetch failed', err);
  }

  const callouts: LandingCallout[] = [
    {
      label: 'Reads two things',
      value: 'Game.log',
      sub: 'and rsi.com, as you, if you ask',
    },
    {
      label: 'EAC-safe by construction',
      value: 'No hooks',
      sub: 'No injection · no memory reads',
      tone: 'good',
    },
    { label: 'Licence', value: 'MPL-2.0', sub: 'Open-source client' },
    ...(version
      ? [
          {
            label: 'Live channel',
            value: `v${version}`,
            sub: 'Windows · Linux · auto-update',
          },
        ]
      : []),
    { label: 'Stays local', value: 'Until you say', sub: 'Sync is opt-in, per device' },
    { label: 'Never collected', value: 'Chat', sub: 'Nor other players, nor game files' },
  ];

  return (
    <LandingProjection
      calibration={calibration}
      nav={navSections({ signedIn: false }, 'home')}
      tagline="Track your Star Citizen play."
      words={HERO_WORDS}
      detail="Reads the log the game already writes. Stays on your PC until you turn on sync."
      callouts={callouts}
      onCalibrate={async (id: string) => {
        'use server';
        await setCalibrationAction(id);
      }}
    >
      <Pane
        variant="static"
        title="What you get"
        ctx="A telemetry tool, not a fan shrine"
        trailing={version ? <BeamChip dot>Live channel v{version}</BeamChip> : undefined}
      >
        {/* Countable properties of the product, not usage figures: the lens
            count is `lib/lens.ts`, the calibrations are the four the chrome
            offers, and emitters are unlimited by design. */}
        <SubStats
          items={[
            { k: 'Lenses', v: '6' },
            { k: 'Calibrations', v: '4' },
            { k: 'Emitters', v: 'Unlimited' },
            { k: 'Licence', v: 'MPL-2.0' },
          ]}
        />
        <div className="hp-landing-grid">
          {FEATURES.map((f) => (
            <Plane key={f.title} tilt="flat" cap={f.title}>
              <p className="hp-prose">{f.body}</p>
            </Plane>
          ))}
        </div>
      </Pane>

      <Pane
        variant="static"
        title="What it does not do"
        ctx="If a feature needs any of these, it isn’t built"
      >
        <Plane tilt="flat" cap="Excluded by design" hint="from the repository README">
          {EXCLUSIONS.map((r, i) => (
            <MeterRow key={r} rank={i + 1} name={r} value="never" valueText />
          ))}
        </Plane>
      </Pane>

      {/* The product's own attribution — shared, so the words exist once. */}
      <SiteLegalPlate version={version ? `v${version} live` : undefined} />
    </LandingProjection>
  );
}
