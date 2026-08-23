'use client';

import React from 'react';
import { useRouter } from 'next/navigation';
import { chromeLink } from '@/components/projection/chromeLink';
import type { Route } from 'next';
import {
  Projection,
  Depth,
  ChromeBar,
  Ring,
  BrandHero,
  Callout,
  CalloutField,
  BeamButton,
  slotFor,
  type Calibration,
  type CalibrationId,
  type NavSection,
} from 'holo';

/**
 * `/` — the landing page, in the projection.
 *
 * GROUNDED. `HoloLanding.jsx` is one of only three web screens COVERAGE marks
 * as read from real source — it was built from this very route — so the shape
 * below follows the kit closely, unlike every other port in this document.
 *
 * `surface="brand"` is REQUIRED, not stylistic: it is what opens the ring to
 * `min(760px, 72vw)`, and `BrandHero` is sized from the ring. Without it the
 * hero overflows the circle.
 *
 * WHERE THE KIT AND THE PRODUCT DISAGREE, THE PRODUCT WINS:
 *
 *   - The rotating words. The kit uses `['Deaths.', 'Jumps.', 'Contracts.',
 *     'Sessions.']` and its prompt claims those match "the product's own hero".
 *     They do not — the shipped hero rotates `StarStats. / Your manifest. /
 *     Your numbers. / Your timeline.`, a possessive claim rather than a list of
 *     event types. The real ones are used; see `HERO_WORDS` in `page.tsx`.
 *   - The callout figures. The kit's read `92,481 events read`, `v1.3.2`,
 *     `Six panes`. Those are illustrative, and a landing page has no reader
 *     data to draw on — so this ships ONLY facts it can source: the live
 *     version comes from the release feed the Emitter page already reads, and
 *     the rest are properties of the repository. Nothing here is a figure
 *     invented to fill a slot.
 *   - The legal plate. See `LegalPlate` — the kit's default disclaimer and the
 *     product's shipped attribution are different texts, and the shipped one is
 *     passed verbatim.
 */
export interface LandingCallout {
  label: string;
  value: string;
  sub: string;
  tone?: 'good';
}

export function LandingProjection({
  calibration,
  nav,
  words,
  tagline,
  detail,
  callouts,
  onCalibrate,
  children,
}: {
  calibration: Calibration;
  nav: NavSection[];
  words: readonly string[];
  tagline: string;
  detail: string;
  callouts: readonly LandingCallout[];
  onCalibrate: (id: string) => void | Promise<void>;
  /** The reading half, docked below the volume. */
  children: React.ReactNode;
}) {
  const router = useRouter();
  const [cal, setCal] = React.useState<Calibration>(calibration);
  React.useEffect(() => setCal(calibration), [calibration]);
  const [recalKey, setRecalKey] = React.useState(0);

  const calibrate = (id: CalibrationId) => {
    setCal(id);
    setRecalKey((k) => k + 1);
    onCalibrate(id);
  };

  // The product's real lenses (`lib/lens.ts`). "All" is a filter, not a
  // subject, so it is not a segment — the same reasoning the kit applies.
  //
  // Equal shares, and that is a STATED LIMIT rather than a guess dressed as
  // data: a signed-out visitor has no timeline, so there is no distribution to
  // weight these by. The ring is showing what the product measures, not how
  // much of it anyone did.
  const segments = React.useMemo(
    () =>
      ['Activity', 'Travel', 'Combat', 'Loadout', 'Commerce'].map((name) => ({
        name,
        share: 1 / 5,
      })),
    [],
  );

  return (
    <div className="ss-projection-root">
      <Projection
        surface="brand"
        calibration={cal}
        recalKey={recalKey}
        chrome={
          <ChromeBar
            renderLink={chromeLink}
            // No handle: this route redirects a signed-in reader to `/me`, so
            // the chrome here is always the signed-out one.
            live
            clock="Unofficial"
            calibration={cal}
            onCalibrate={calibrate}
            sections={nav}
            onSignIn={() => router.push('/auth/login' as Route)}
            onNavigate={(id) => router.push(`/${id}` as Route)}
          />
        }
        hint="Unofficial · community project"
        style={{ overflowY: 'auto' }}
      >
        <Depth depth={20}>
          <Ring
            mode="segments"
            segments={segments}
            onSelectSegment={() => router.push('/features' as Route)}
          />
        </Depth>

        <Depth depth={36}>
          <BrandHero tagline={tagline} words={words} detail={detail} />
        </Depth>

        <Depth depth={54}>
          <CalloutField>
            {/* `slotFor` rather than hand-placed coordinates: the six fixed
                slots are what keep callouts off the ring stroke and out of each
                other, and the slot owns which SIDE it is on — passing a side
                separately would let the two disagree.

                CAPACITY IS REAL. `CalloutField` caps at six and hides the whole
                field below 1180px, so nothing may live ONLY here. Every fact
                below is also stated in the reading half or the legal plate. */}
            {callouts.slice(0, 6).map((c, i) => {
              const slot = slotFor(i);
              if (!slot) return null;
              return (
                <Callout
                  key={c.label}
                  side={slot.side}
                  at={slot.at}
                  tone={c.tone}
                  label={c.label}
                  value={c.value}
                  sub={c.sub}
                />
              );
            })}
          </CalloutField>

          <div className="hp-cta">
            <BeamButton
              variant="primary"
              onClick={() => router.push('/auth/signup' as Route)}
            >
              Create account →
            </BeamButton>
            <BeamButton
              variant="ghost"
              onClick={() => router.push('/downloads' as Route)}
            >
              Download tray client
            </BeamButton>
          </div>
        </Depth>

        {/* The reading half of the landing page, docked below the volume. */}
        <div className="hp-landing-read">{children}</div>
      </Projection>
    </div>
  );
}
