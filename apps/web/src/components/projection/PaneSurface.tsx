'use client';

import React from 'react';
import { useRouter } from 'next/navigation';
import { chromeLink } from '@/components/projection/chromeLink';
import type { Route } from 'next';
import { SiteLegalPlate } from './SiteLegalPlate';
import { useShellData } from './ShellData';
import {
  Projection,
  Pane,
  LensRail,
  Crumb,
  ChromeBar,
  BeamAlert,
  CalibrationChoice,
  type Calibration,
  type CalibrationId,
  type NavSection,
} from 'holo';

/**
 * The projection's STATIC surface: a scrolling stack of panes, grouped behind
 * the lens rail.
 *
 * `/settings` and `/sharing` are the same shape — a long document of anchored
 * sections that a reader navigates rather than an instrument they read — and
 * the kit builds every non-projection screen this way (`SiteFrame`: a
 * non-parallax volume full of `Pane variant="static"`). This is that, shared,
 * so the two do not drift.
 *
 * SECTION IDS ARE BEHAVIOUR on both surfaces. Server actions redirect to a
 * fragment (`#security`, `#danger`, `#rsi`, `#share-editor`), and `/settings/2fa`
 * redirects to `#security`. Grouping means the target section is not mounted
 * until its group is, so the rail is driven from `location.hash` — that wiring
 * is the price of the rail and it lives here, once.
 */
export interface SurfaceSection {
  /** Anchor id — load-bearing. Never rename without the redirects. */
  id: string;
  /**
   * ReactNode, not string: a segment whose heading depends on the ROUTE has to
   * pass a client component, because a layout cannot read the pathname.
   * `/auth/**` is that case — nine routes, one layout, a different step each.
   * Only `Pane`'s title consumes this, and it takes a node.
   */
  title: React.ReactNode;
  ctx?: React.ReactNode;
  /** Which rail group owns it. */
  group: string;
  /** Server-rendered content. */
  node: React.ReactNode;
  /**
   * Extra fragment ids that live INSIDE this section and should select its
   * group. `/sharing` needs this: the edit flow redirects to
   * `#share-editor`, which is a form within the outbound section rather than
   * a section of its own, and without it the rail would not open the group
   * that contains the editor being scrolled to.
   */
  anchors?: readonly string[];
  /**
   * Client control this section owns, rendered above `node`. The calibration
   * picker cannot be built server-side: it recalibrates IN PLACE rather than
   * posting and reloading, and that is client state.
   */
  slot?: 'calibration';
}

export interface SurfaceGroup {
  key: string;
  label: string;
}

export interface PaneSurfaceProps {
  /**
   * The signed-in reader's handle, or undefined for a PUBLIC surface.
   *
   * `/kb` is browsable signed-out, so the chrome has to work without one:
   * `ChromeBar` renders a Sign in action instead of the account menu, and the
   * nav must already have been filtered with `navFor({ signedIn: false })` —
   * a signed-out visitor should not see the labels of pages they cannot open.
   */
  handle?: string;
  calibration: Calibration;
  nav: NavSection[];
  /** Trail back out of this surface. */
  crumb: { label: string; href?: string }[];
  groups: readonly SurfaceGroup[];
  sections: readonly SurfaceSection[];
  /** Resolved copy for `?status=` / `?error=`, mapped server-side. */
  notice: { tone: 'good' | 'bad' | 'warn'; message: React.ReactNode } | null;
  /** Rendered above every group — a degraded-service banner, say. */
  banner?: React.ReactNode;
  onCalibrate: (id: CalibrationId) => void;
  /** No-JS fallback target for the calibration picker's form. */
  themeAction?: (formData: FormData) => void | Promise<void>;
  account?: { id: string; label: string; href?: string }[];
  /**
   * Chrome control slot — the range tabs on a windowed surface.
   *
   * Kept as a slot rather than a `range` prop because not every static surface
   * is windowed: `/settings` and `/devices` show state, not a period, and a
   * range control on them would imply the page could be scoped when it cannot.
   */
  chromeTrailing?: React.ReactNode;
  /**
   * Let the last crumb step carry the page's `<h1>`.
   *
   * TRUE by default, because the projection has no page title of its own and
   * every flat screen these replaced had one. Pass `false` on a surface whose
   * CONTENT already renders an h1 — the admin console and the static marketing
   * pages both do, and theirs name the specific page far better than a shared
   * crumb could. Getting it wrong ships two h1s.
   */
  crumbHeading?: boolean;
  /**
   * Append the CIG trademark plate below the sections.
   *
   * The design system requires it on every STATIC/public surface, and the flat
   * shell used to supply it from `layout.tsx`'s signed-out footer. The
   * projection hides that footer, so a public surface carries the notice here
   * or not at all — which is exactly what happened: after the first wave of
   * ports, only the landing page still showed it.
   *
   * DEFAULTS TO TRUE. It was opt-in for public routes at first, on the reasoning
   * that a signed-in dashboard is not a static surface — but the flat
   * `.ss-app-footer` carried the same obligation for signed-in pages (its own
   * comment cites brand book §11) and the projection hides that too. Both
   * audiences lost it; both get it back. Pass `legal={false}` only for a
   * surface that genuinely has another copy of the plate on screen.
   */
  legal?: boolean;
}

export function PaneSurface({
  handle,
  calibration,
  nav,
  crumb,
  groups,
  sections,
  notice,
  banner,
  onCalibrate,
  themeAction,
  account,
  chromeTrailing,
  crumbHeading = true,
  legal = true,
}: PaneSurfaceProps) {
  const router = useRouter();
  const { inboundShares } = useShellData();

  /**
   * The inbound-share badge, put on the Sharing entry centrally.
   *
   * Every shell builds its own `account` array, so decorating here rather than
   * at each call site is what keeps the badge from appearing on some surfaces
   * and not others — which is the failure mode that makes a notification worse
   * than useless.
   */
  const accountItems = React.useMemo(
    () =>
      account?.map((a) =>
        a.id === 'sharing' && inboundShares > 0
          ? { ...a, badge: inboundShares }
          : a,
      ),
    [account, inboundShares],
  );
  const [group, setGroup] = React.useState(0);
  const [recalKey, setRecalKey] = React.useState(0);
  // Local beam state, NOT the server prop. The persist action deliberately
  // does not revalidate (re-rendering every section to change a hue would be
  // absurd), so rendering the prop meant the pips fired the shock ring and
  // scan wipe over a volume that stayed the old colour until the next
  // navigation: the recalibration event played and nothing recalibrated.
  const [cal, setCal] = React.useState<Calibration>(calibration);
  React.useEffect(() => setCal(calibration), [calibration]);
  const scrollRef = React.useRef<HTMLDivElement | null>(null);

  /**
   * A GROUP WITH NO SECTIONS IS NOT OFFERED.
   *
   * Surfaces declare their groups statically and build their sections from
   * data, so a reader with nothing in one gets a lens that lights up and shows
   * an empty volume. `/me/travel` shipped exactly that: its Trail lens is gated
   * on having any stops, and a reader without them saw the rail item and no
   * panes under it. It is the same shape as the Emitter's blank surface, and
   * the same lesson — a control that does nothing is worse than an absent one.
   *
   * Filtering here rather than at each call site because every surface that
   * builds sections conditionally has the bug latent in it.
   */
  const liveGroups = React.useMemo(
    () => groups.filter((g) => sections.some((s) => s.group === g.key)),
    [groups, sections],
  );

  const groupOfSection = React.useMemo(() => {
    const m = new Map<string, number>();
    sections.forEach((s) => {
      const i = liveGroups.findIndex((g) => g.key === s.group);
      if (i < 0) return;
      m.set(s.id, i);
      s.anchors?.forEach((a) => m.set(a, i));
    });
    return m;
  }, [sections, groups]);

  /**
   * Drive the rail from the URL fragment.
   *
   * `hashchange` is handled as well as mount: a server action redirects to the
   * same path with a new fragment, which does not always remount this tree.
   */
  const openHash = React.useCallback(() => {
    const id = window.location.hash.replace(/^#/, '');
    if (!id) return;
    const g = groupOfSection.get(id);
    if (g === undefined) return;
    setGroup(g);
    // The section mounts in the same commit as the group change, so wait a
    // frame before measuring it.
    requestAnimationFrame(() => {
      document
        .getElementById(id)
        ?.scrollIntoView({ block: 'start', behavior: 'auto' });
    });
  }, [groupOfSection]);

  React.useEffect(() => {
    openHash();
    window.addEventListener('hashchange', openHash);
    return () => window.removeEventListener('hashchange', openHash);
  }, [openHash]);

  // A group change is a new reading position, not a continuation of the last
  // one — start it at the top rather than wherever the previous group sat.
  const selectGroup = (i: number) => {
    setGroup(i);
    scrollRef.current?.scrollTo({ top: 0 });
  };

  const calibrate = (id: CalibrationId) => {
    setCal(id);
    setRecalKey((k) => k + 1);
    onCalibrate(id);
  };

  const activeGroup = liveGroups[Math.min(group, liveGroups.length - 1)];
  const shown = sections.filter((s) => s.group === activeGroup?.key);

  return (
    <div className="ss-projection-root">
      <Projection
        calibration={cal}
        recalKey={recalKey}
        // NOT a new `surface`. The system declares exactly three (default,
        // brand, console); a fourth for these pages would be invented
        // vocabulary. Parallax is off because this is a reading-and-typing
        // surface: the effect exists to make a volume feel inhabited, and
        // under a form it just moves the field you are aiming at.
        parallax={false}
        chrome={
          <ChromeBar
            renderLink={chromeLink}
            handle={handle}
            calibration={cal}
            onCalibrate={calibrate}
            sections={nav}
            trailing={chromeTrailing}
            account={handle ? accountItems : undefined}
            onSignIn={
              handle ? undefined : () => router.push('/auth/login' as Route)
            }
            // "Projection live" claims an uplink is streaming. A signed-out
            // visitor has no uplink, so the chrome must not say so.
            live={Boolean(handle)}
            onNavigate={(id) => router.push(`/${id}` as Route)}
          />
        }
        crumb={
          <Crumb
            // The last crumb step is this page's name, so it carries the h1 —
            // the projection otherwise has no page heading, and every flat
            // screen these replaced had one. Surfaces whose content brings its
            // own h1 opt out; see `crumbHeading`.
            heading={crumbHeading}
            parts={crumb.map((c) => ({
              t: c.label,
              onClick: c.href
                ? () => router.push(c.href as Route)
                : undefined,
            }))}
          />
        }
        // A one-group surface has nothing to navigate BETWEEN, and a rail with
        // a single lit item reads as a control that does not work. `/me/loadout`
        // is the case: the paperdoll and the carried gear are one view of one
        // kit, and splitting them behind a rail would be worse than the flat
        // page it replaces.
        lens={
          liveGroups.length > 1 ? (
            <LensRail
              lenses={liveGroups.map((g) => ({ id: g.key, name: g.label }))}
              active={group}
              onSelect={selectGroup}
            />
          ) : undefined
        }
      >
        <div className="hp-settings" ref={scrollRef}>
          <div className="hp-settings__inner">
            {notice ? (
              <BeamAlert tone={notice.tone}>{notice.message}</BeamAlert>
            ) : null}
            {banner}

            {shown.map((s) => (
              // `scroll-margin-top` keeps a fragment jump clear of the chrome.
              <div key={s.id} id={s.id} style={{ scrollMarginTop: 24 }}>
                <Pane variant="static" title={s.title} ctx={s.ctx}>
                  {s.slot === 'calibration' ? (
                    <CalibrationChoice
                      active={cal}
                      formAction={themeAction}
                      onSelect={calibrate}
                    />
                  ) : null}
                  {/* `React.Children.toArray` and not a bare `{s.node}`.
                      Section content is built in a SERVER component and handed
                      to this client one as a prop, and a Fragment root does not
                      survive that boundary as a Fragment — it serialises to a
                      keyless array. React only notices on a client re-render,
                      so it shows up as a key warning the moment a reader
                      switches groups and never on first paint. `toArray`
                      assigns the keys, which is what it is for. */}
                  {React.Children.toArray(s.node)}
                </Pane>
              </div>
            ))}
            {legal ? <SiteLegalPlate /> : null}
          </div>
        </div>
      </Projection>
    </div>
  );
}
