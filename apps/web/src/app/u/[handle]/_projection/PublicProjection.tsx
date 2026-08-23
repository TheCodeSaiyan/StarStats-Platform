'use client';

import React from 'react';
import { useRouter } from 'next/navigation';
import type { Route } from 'next';
import {
  Projection,
  ChromeBar,
  Crumb,
  LensRail,
  Depth,
  Ring,
  CoreReadout,
  CalloutField,
  Callout,
  slotFor,
  Pane,
  SubStats,
  Plane,
  MeterRow,
  Flatline,
  type Calibration,
  type CalibrationId,
  type NavSection,
} from 'holo';
import { chromeLink } from '@/components/projection/chromeLink';
import { SiteLegalPlate } from '@/components/projection/SiteLegalPlate';

/**
 * `/u/[handle]` — the public profile, as a volume.
 *
 * WHY THIS IS NOT `PaneSurface`. `Profile.jsx` is a ring, a core readout and a
 * callout field, the same shape as `Holotable` — the page a stranger reads is
 * a projection of a pilot, not a settings sheet. The port previously framed it
 * with the static surface and left the flat widget canvas inside, and the note
 * on `ProfileProjection` said the blocker was a public-scoped element
 * catalogue. THAT WAS WRONG, and the correction matters more than the layout:
 * `GET /v1/public/{handle}/share-scopes` is unauthenticated and returns the
 * pilot's own per-scope switches. The page already fetched it and handed it
 * straight to `WidgetCanvas` without ever reading it.
 *
 * WHY THIS IS NOT `MeProjection` EITHER, which COVERAGE states outright: no
 * layout editor, no range control, no ring map. A visitor is not arranging
 * anything and has no window to choose — offering either would be chrome that
 * does nothing.
 *
 * THE RING SHOWS EVENT TYPES, NOT LENSES — a departure from the kit, with a
 * reason. `Profile.jsx` draws one segment per published lens at `1/n` each,
 * which is fine in a mock and dishonest here: equal segments draw a
 * distribution that does not exist, and a reader takes a ring to be
 * proportional because everywhere else in this product it is. `by_type` IS a
 * real public distribution, so the ring carries that, and the published set is
 * stated where a set belongs — in the pane, and in a callout.
 *
 * WHAT IS DELIBERATELY ABSENT. The kit's callouts include locations seen,
 * sessions shared, quantum transits and kill/death. `PublicSummaryResponse` is
 * `{ claimed_handle, total, by_type, supporter }` — none of those four are in
 * it, and inventing them on the page a stranger reads is the worst place in
 * the product to guess. Six honest callouts, or fewer.
 */
export interface PublicCalloutVM {
  id: string;
  label: string;
  value: string;
  sub?: string;
  tone?: 'warn' | 'bad';
}

export interface PublicProjectionProps {
  /** The profile's owner. Never used for the chrome. */
  subject: string;
  /**
   * The VIEWER's handle, or null. Kept separate from `subject` on purpose:
   * putting the subject's handle in the chrome would tell a stranger they were
   * signed in as someone else.
   */
  handle: string | null;
  kind: 'public' | 'shared' | 'self';
  calibration: Calibration;
  nav: NavSection[];
  crumb: { label: string; href?: string }[];
  /** Core readout: the pilot's shared total. */
  total: string;
  totalDetail: string;
  /** Ring segments — real shares of `total`, never a placeholder split. */
  segments: { name: string; share: number }[];
  callouts: PublicCalloutVM[];
  /** Pane header figures. */
  subStats: { k: string; v: string; tone?: 'warn' | 'bad' }[];
  /** Scopes this pilot publishes, and the ones they do not. */
  published: string[];
  withheld: string[];
  /** True when the share scopes were actually read; false if the fetch failed. */
  scopesKnown: boolean;
  /** Server-rendered: supporter chip and badges. */
  chips: React.ReactNode;
  /** Server-rendered: profile card, widget canvas, visibility notice. */
  body: React.ReactNode;
  onCalibrate: (id: CalibrationId) => void;
}

const LENSES = [
  { id: 'overview', name: 'Overview' },
  { id: 'sessions', name: 'Sessions' },
  { id: 'entities', name: 'Entities' },
];

export function PublicProjection({
  subject,
  handle,
  kind,
  calibration,
  nav,
  crumb,
  total,
  totalDetail,
  segments,
  callouts,
  subStats,
  published,
  withheld,
  scopesKnown,
  chips,
  body,
  onCalibrate,
}: PublicProjectionProps) {
  const router = useRouter();
  const [cal, setCal] = React.useState<Calibration>(calibration);
  const [recalKey, setRecalKey] = React.useState(0);

  const calibrate = React.useCallback(
    (id: CalibrationId) => {
      setCal(id as Calibration);
      setRecalKey((k) => k + 1);
      onCalibrate(id);
    },
    [onCalibrate],
  );

  return (
    <div className="ss-projection-root">
      <Projection
        mode="overview"
        calibration={cal}
        recalKey={recalKey}
        chrome={
          <ChromeBar
            renderLink={chromeLink}
            handle={handle ?? undefined}
            calibration={cal}
            onCalibrate={calibrate}
            sections={nav}
            // `live` claims an uplink is streaming to THIS screen. Reading
            // someone else's profile is not that, whoever you are.
            live={false}
            account={
              handle
                ? [
                    { id: 'me', label: 'Projection', href: '/me' },
                    { id: 'sharing', label: 'Sharing', href: '/sharing' },
                    { id: 'settings', label: 'Calibrate', href: '/settings' },
                  ]
                : undefined
            }
            onSignIn={
              handle ? undefined : () => router.push('/auth/login' as Route)
            }
          />
        }
        crumb={
          <Crumb
            // The h1 is inside the pane header below, naming the handle.
            heading={false}
            parts={crumb.map((c) => ({
              t: c.label,
              onClick: c.href ? () => router.push(c.href as Route) : undefined,
            }))}
          />
        }
        lens={
          <LensRail
            lenses={LENSES}
            active={0}
            // These are REAL ROUTES, not client state — `/u/x/sessions` and
            // `/u/x/entities` exist and are already ported. The rail navigates
            // rather than swapping a local view, so a lens is shareable and
            // the back button works.
            onSelect={(i) => {
              const id = LENSES[i]?.id;
              if (!id || id === 'overview') return;
              router.push(`/u/${subject}/${id}` as Route);
            }}
          />
        }
        hint="Read-only · shows only what this pilot published"
      >
        <Depth depth={20}>
          <Ring mode="segments" segments={segments} activeIndex={-1} />
        </Depth>

        <Depth depth={36}>
          <CoreReadout
            value={total}
            label="Events shared by this pilot"
            detail={totalDetail}
          />
        </Depth>

        <Depth depth={54}>
          <CalloutField>
            {callouts.map((c, i) => {
              const slot = slotFor(i);
              return (
                <Callout
                  key={c.id}
                  label={c.label}
                  value={c.value}
                  sub={c.sub}
                  tone={c.tone}
                  side={slot?.side ?? 'l'}
                  at={slot?.at}
                />
              );
            })}
          </CalloutField>
        </Depth>
      </Projection>

      {/* DOCKED BELOW THE VOLUME, not inside it.

          `.hp-pane` is `opacity: 0; pointer-events: none` until the stage is
          in `data-mode="detail"` — the pane is the thing a LENS opens. This
          screen has no in-page lens to open (Sessions and Entities are real
          routes), so a pane rendered inside an overview volume is invisible
          and inert. It rendered anyway, complete with the h1 and the whole
          widget canvas, and `toBeVisible()` did not catch it: Playwright's
          visibility check reads the box and `visibility`, NOT opacity.

          `variant="static"` is the docked form the system already has for
          exactly this — a pane in normal page flow — and it is what every
          static surface uses. The volume carries the headline; the detail
          reads underneath it, the same way `/me` docks its legal plate. */}
      <div className="hp-volume-below">
        <Pane
          variant="static"
          // The pane header IS this page's heading — the volume has no
          // other titled surface. `level` makes it an h1 rather than
          // nesting one inside the header's h2.
          level={1}
          title={`@${subject}`}
          ctx={
            kind === 'self'
              ? 'Your profile, as others see it'
              : kind === 'shared'
                ? 'Shared with you'
                : 'Public projection'
          }
          trailing={chips}
        >
          <SubStats items={subStats} />

          {/* Both halves, always — `Profile.jsx`: "a public profile must
              never imply data it is not allowed to show." Hiding the
              withheld set would let a sparse page read as a quiet pilot
              rather than a private one. */}
          <Plane
            cap={<h2>Published</h2>}
            hint="set by this pilot"
            style={{ marginTop: 18 }}
          >
            {!scopesKnown ? (
              <Flatline
                compact
                reason="no-signal"
                title="Could not read what this pilot publishes"
                hint="The scopes endpoint did not answer. Nothing is being withheld or implied here."
              />
            ) : published.length > 0 ? (
              published.map((name, i) => (
                <MeterRow
                  key={name}
                  rank={i + 1}
                  name={name}
                  value="public"
                  valueText
                />
              ))
            ) : (
              <Flatline
                compact
                reason="no-signal"
                title="Nothing is published"
                hint="This pilot shares their handle and nothing else."
              />
            )}
          </Plane>

          {scopesKnown && withheld.length > 0 ? (
            <Plane
              tilt="flat"
              cap={<h2>Not published</h2>}
              style={{ marginTop: 14 }}
            >
              <Flatline
                compact
                reason="no-signal"
                title={`${withheld.join(', ')} ${withheld.length === 1 ? 'is' : 'are'} private`}
                hint={
                  kind === 'self'
                    ? 'Only you can change what this page shows.'
                    : 'Only this pilot can change what this page shows.'
                }
              />
            </Plane>
          ) : null}

          {body}
        </Pane>

        <SiteLegalPlate />
      </div>
    </div>
  );
}
