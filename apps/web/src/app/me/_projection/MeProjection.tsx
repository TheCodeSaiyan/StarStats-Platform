'use client';

import React from 'react';
import Link from 'next/link';
import { bucketSeries } from './series';
import { SiteLegalPlate } from '@/components/projection/SiteLegalPlate';
import { useShellData } from '@/components/projection/ShellData';
import { useRouter } from 'next/navigation';
import { chromeLink } from '@/components/projection/chromeLink';
import type { Route } from 'next';
import {
  Projection,
  Depth,
  Ring,
  CoreReadout,
  Callout,
  CalloutField,
  Pane,
  SubStats,
  LensRail,
  Crumb,
  RangeTabs,
  ChromeBar,
  LayoutEditor,
  Flatline,
  BeamTip,
  useLayout,
  equalShares,
  slotFor,
  type Calibration,
  type CalibrationId,
  type MapLayout,
  type NavSection,
  type SubStatItem,
  Trace,
} from 'holo';
import { widgetMatchesLens, type Lens } from '@/lib/lens';
import { RAIL_LENSES } from './rail';
import type { RangeId } from '@/lib/range';
import type { WidgetId } from '@/app/_components/widgets/types';
import type { CalloutVM } from './elements';
import { PROJECTION_CATALOGUE } from './catalogue';

/**
 * `/me` — the reader's own projection.
 *
 * Replaces the flat widget dashboard (identity header + RangeBar + 24-column
 * drag/resize grid). What each piece became is recorded in
 * docs/PLAN-PROJECTION-PORT.md; the load-bearing ones here:
 *
 *  - The lifetime identity figures moved into the CHROME, because they are
 *    range-INDEPENDENT and everything in the volume below is range-scoped.
 *    Keeping them in the callout field would have put two time bases side by
 *    side under one visual rule.
 *  - The range control stays a URL param (`?range=`), not component state, so
 *    the server components keep re-querying and the view stays shareable and
 *    back-button correct. `RangeTabs` therefore renders Next <Link>s.
 *  - Reader-controlled tile GEOMETRY is gone. The reader still chooses which
 *    elements project and in what order (`LayoutEditor`), persisted on the
 *    account — never localStorage, which would silently downgrade a working
 *    cross-device behaviour to a per-device one.
 */

export interface LensPaneVM {
  id: WidgetId;
  node: React.ReactNode;
}

export interface MeProjectionProps {
  handle: string;
  supporterTier: string | null;
  enlistmentYear: string | null;
  /** Range-independent lifetime figures, shown in the chrome. */
  lifetime: {
    playtime: string;
    events: string;
    locations: string;
    kd: string;
    /** Derivation for K/D when some deaths were reconstructed. */
    kdNote?: string;
  };
  calibration: Calibration;
  range: RangeId;
  /** Element ids the reader has enabled, in their saved order. */
  enabledIds: string[];
  callouts: CalloutVM[];
  /** Server-rendered pane content, keyed by element id. */
  planes: LensPaneVM[];
  ringMap: MapLayout;
  /**
   * Real daily event counts behind the trace and the ring's bars.
   *
   * `Holotable.jsx` puts a `Trace` in every lens's detail pane and switches the
   * ring to `bars` when a lens is open; both were missing from this screen. The
   * kit generates their series from a seed because it is a mock — passing a
   * generated one here would draw a chart of nothing and label it the reader's
   * own history, so an empty array renders neither.
   */
  traceValues: number[];
  traceDays: number;
  nav: NavSection[];
  /** Persists the layout to the account. */
  onSaveLayout: (ids: string[]) => void;
  /** Persists the calibration (reuses the existing theme preference). */
  onCalibrate: (id: CalibrationId) => void;
}


export function MeProjection({
  handle,
  supporterTier,
  enlistmentYear,
  lifetime,
  calibration,
  range,
  enabledIds,
  callouts,
  planes,
  ringMap,
  traceValues,
  traceDays,
  nav,
  onSaveLayout,
  onCalibrate,
}: MeProjectionProps) {
  const router = useRouter();
  const { inboundShares } = useShellData();
  const [lens, setLens] = React.useState(-1);
  const [record, setRecord] = React.useState<string | null>(null);
  const [editing, setEditing] = React.useState(false);
  const [recalKey, setRecalKey] = React.useState(0);
  // The beam has to be LOCAL state, not the server prop.
  //
  // `setCalibrationAction` writes the cookie and the preference but does not
  // revalidate (a full re-render of every element to change a hue would be
  // absurd), so the prop cannot change in response to a click. Rendering it
  // directly meant the pips fired the shock ring and scan wipe over a volume
  // that stayed the old colour until the next navigation — the recalibration
  // event played, and nothing recalibrated.
  const [cal, setCal] = React.useState<Calibration>(calibration);
  // Keep in step if the server sends a different value on a later navigation.
  React.useEffect(() => setCal(calibration), [calibration]);

  const layout = useLayout('me.projection', PROJECTION_CATALOGUE, {
    initial: enabledIds,
    persist: onSaveLayout,
  });

  const mode = record ? 'inspect' : lens > -1 ? 'detail' : 'overview';
  const activeLens: Lens | null = lens > -1 ? RAIL_LENSES[lens].id : null;

  const openLens = React.useCallback((i: number) => {
    setLens(i);
    setRecord(null);
  }, []);
  const toOverview = React.useCallback(() => {
    setLens(-1);
    setRecord(null);
  }, []);

  // 1–5 pick a lens, E toggles the layout editor, Esc walks one depth out —
  // the same bindings the design system documents for the rail.
  React.useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      // Never hijack a key the reader is typing into a field.
      const t = e.target;
      if (t instanceof HTMLElement) {
        const tag = t.tagName;
        if (tag === 'INPUT' || tag === 'TEXTAREA' || t.isContentEditable) return;
      }
      if (e.key === 'Escape') {
        if (record) setRecord(null);
        else toOverview();
        return;
      }
      if (/^[1-6]$/.test(e.key)) openLens(Number(e.key) - 1);
      if (e.key.toLowerCase() === 'e') setEditing((v) => !v);
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [record, openLens, toOverview]);

  const calibrate = (id: CalibrationId) => {
    // Recalibration is an EVENT, not a repaint: repaint the beam AND bump the
    // key so the shock ring, scan wipe and emitter surge fire with it.
    setCal(id);
    setRecalKey((k) => k + 1);
    onCalibrate(id);
  };

  /* ── Ring ────────────────────────────────────────────────────────────────
   * Segment widths are meant to carry each lens's share of captured events.
   * That needs a per-event lens attribution the server does not send yet, so
   * segments are EQUAL for now and the ring navigates rather than charting. A
   * proportion invented from mixed units (hours vs jumps vs a ratio vs credits)
   * would be a confidently wrong number, which is worse than an absent one. */
  const segments = equalShares(RAIL_LENSES.map((l) => ({ name: l.label })));
  const showMap = activeLens === 'travel' && layout.has('journey');
  /**
   * `Holotable.jsx`: segments on the overview, `map` for the travel lens, and
   * `bars` for any other open lens. The third case was missing — an open lens
   * kept the overview's segment ring, so the ring said nothing about the lens
   * you had just opened.
   *
   * Bars only when there is a real series to draw. `Ring` would happily render
   * an empty set as a flat field, which reads as "no activity" rather than "no
   * data" — two different claims.
   */
  const ringBars = React.useMemo(
    () => bucketSeries(traceValues, 24),
    [traceValues],
  );
  /*
   * The condition is "a SPECIFIC lens is open", not "the lens is not All".
   * `activeLens` is `null` on the overview — no lens selected — and `null !==
   * 'all'` is true, so the first version put the overview into bars and the
   * segment ring never appeared at all. Caught by a test asserting the overview
   * still has segments; nothing else would have.
   *
   * "All" is a filter rather than a subject, so it keeps the segment ring too.
   */
  const lensIsSubject = activeLens !== null && activeLens !== 'all';
  const ringMode =
    showMap && ringMap.nodes.length > 0
      ? 'map'
      : lensIsSubject && ringBars.length > 0
        ? 'bars'
        : 'segments';

  /* ── Callouts ────────────────────────────────────────────────────────────
   * Range-scoped only. Positions come from the six fixed slots, filled in the
   * reader's layout order; CalloutField caps at six and reports the rest as
   * "+N more in layout" rather than drawing them over the ring. */
  const projectedIds = layout.projected.map((e) => e.id);
  const shownCallouts = callouts
    .filter((c) => projectedIds.includes(c.id))
    .sort((a, b) => projectedIds.indexOf(a.id) - projectedIds.indexOf(b.id));

  /* ── Lens panes ──────────────────────────────────────────────────────────
   * A lens HIDES elements, it never changes them, so a figure cannot disagree
   * with itself between lenses.
   *
   * Membership comes from `widgetMatchesLens`, the product's own map, NOT from
   * a copy in the catalogue. The copy drifted the moment it existed. */
  /**
   * THE CENTRE FOLLOWS THE LENS.
   *
   * `Holotable.jsx` is explicit about this — `const core = L ? L : OV` — so the
   * figure at the centre of the volume is the OPEN LENS's headline, and the
   * lifetime anchor only when nothing is open. This screen showed logged flight
   * time no matter which lens you opened, which makes the ring look like
   * decoration: you select Combat and the middle of the screen keeps reporting
   * hours flown.
   *
   * Sourced from the CALLOUTS, which are already the per-widget headline
   * figures, already built for this render, and already mapped to lenses by
   * `WIDGET_LENSES`. No new fetch, and no second definition of "the headline
   * for Combat" that could disagree with the callout beside it.
   *
   * The order within a lens is a preference, not a ranking: the first enabled
   * callout wins, so a reader who has turned their K/D callout off still gets a
   * Combat centre from whatever they kept.
   */
  const LENS_CORE_PREFERENCE: Partial<Record<Lens, WidgetId[]>> = {
    activity: ['sessions', 'heatmap'],
    travel: ['travel', 'routes'],
    combat: ['lives', 'contracts', 'objectives'],
    commerce: ['spend', 'economy'],
    loadout: ['loadout', 'hangar'],
  };

  const lensCore = React.useMemo(() => {
    if (activeLens == null || activeLens === 'all') return null;
    const order = LENS_CORE_PREFERENCE[activeLens] ?? [];
    for (const id of order) {
      const c = callouts.find((x) => x.id === id);
      if (c) return c;
    }
    // A lens whose callouts are all switched off keeps the lifetime anchor
    // rather than inventing a figure or showing an empty centre.
    return null;
  }, [activeLens, callouts]);

  const lensPlanes = planes.filter(
    (p) => layout.has(p.id) && activeLens != null && widgetMatchesLens(p.id, activeLens),
  );

  const activeLabel = lens > -1 ? RAIL_LENSES[lens].label : null;

  const crumb = [
    { t: 'Overview', onClick: mode === 'overview' ? undefined : toOverview },
    ...(activeLabel
      ? [
          {
            t: activeLabel,
            onClick: record ? () => setRecord(null) : undefined,
          },
        ]
      : []),
    ...(record ? [{ t: record }] : []),
  ];

  return (
    // `role="main"` on a DIV, never a <main> element: globals.css ships a global
    // `main { max-width: 720px }` legacy column for the marketing/auth/legal
    // pages, which would clamp a full-viewport volume into a narrow strip.
    // `.ss-projection-root` is what lets the page step out of the flat app
    // shell — see styles/projection-shell.css.
    // No `id="main"` here: the shell already puts that id on `.ss-main`, and a
    // duplicate would make the skip-link target ambiguous. The projection
    // carries its own skip link to `#hp-content` instead.
    <div className="ss-projection-root">
    <Projection
      mode={mode}
      calibration={cal}
      recalKey={recalKey}
      editing={editing}
      chrome={
        <ChromeBar
            renderLink={chromeLink}
          handle={handle}
          calibration={cal}
          onCalibrate={calibrate}
          sections={nav}
          since={enlistmentYear ? `Citizen since ${enlistmentYear}` : undefined}
          supporter={
            supporterTier ? (
              <span className="hp-chip">{supporterTier} supporter</span>
            ) : undefined
          }
          readouts={
            <>
              <b>{lifetime.playtime}</b> play
              <s />
              <b>{lifetime.events}</b> events
              <s />
              <b>{lifetime.locations}</b> loc
              <s />
              {lifetime.kdNote ? (
                <BeamTip
                  note={lifetime.kdNote}
                  label="How the kill/death ratio is derived"
                >
                  <b>{lifetime.kd}</b>
                </BeamTip>
              ) : (
                <b>{lifetime.kd}</b>
              )}{' '}
              k/d
            </>
          }
          trailing={
            <RangeTabs
              active={range}
              renderItem={(id, label, isActive) => (
                <Link
                  href={`/me?range=${id}` as Route}
                  // `aria-current`, not `aria-pressed`: this is a link to the
                  // current view, not a toggle button.
                  aria-current={isActive ? 'page' : undefined}
                  scroll={false}
                >
                  {label}
                </Link>
              )}
            />
          }
          account={[
            // Editing the layout is this screen's own ACTION, not a
            // destination, so it is intercepted below rather than routed. It
            // belongs here because `E` alone is not discoverable — a reader has
            // no way to learn a bare keystroke exists.
            {
              id: 'edit-layout',
              label: editing ? 'Done editing layout' : 'Edit projection layout',
            },
            { id: 'settings', label: 'Calibrate', href: '/settings' },
            {
              id: 'sharing',
              label: 'Sharing',
              href: '/sharing',
              // The inbound-share badge. `/me` builds its own `ChromeBar`
              // rather than going through `PaneSurface`, so it does not get
              // the central decoration and has to carry it itself.
              badge: inboundShares > 0 ? inboundShares : undefined,
            },
            { id: 'downloads', label: 'Emitter', href: '/downloads' },
          ]}
          onNavigate={(id) => {
            if (id === 'edit-layout') {
              setEditing((v) => !v);
              return;
            }
            router.push(`/${id}` as Route);
          }}
        />
      }
      crumb={<Crumb parts={crumb} />}
      lens={
        <LensRail
          lenses={RAIL_LENSES.map((l) => ({ id: l.id, name: l.label }))}
          active={lens}
          onSelect={openLens}
        />
      }
      hint="Move cursor · 1–6 lenses · E layout · Esc back"
      overlay={
        editing ? (
          <LayoutEditor
            catalogue={PROJECTION_CATALOGUE.map((e) => ({
              id: e.id,
              name: e.name,
              group: e.group,
              hint: e.hint,
            }))}
            layout={layout}
            onClose={() => setEditing(false)}
          />
        ) : null
      }
    >
      <Depth depth={20}>
        <Ring
          mode={ringMode}
          segments={segments}
          activeIndex={lens}
          nodes={ringMap.nodes}
          links={ringMap.links}
          ticks={ringMap.ticks}
          bars={ringBars}
          onSelectSegment={openLens}
          onSelectNode={setRecord}
        />
      </Depth>

      <Depth depth={36}>
        {/* The page's heading.
            The flat /me had `<h1>@handle</h1>` in its identity header, and the
            projection has no titled surface to hang one on — the volume IS the
            page, and this screen's crumb is a depth chain (Overview → lens →
            record) rather than a page name, so it cannot carry the h1 the way
            the static surfaces' crumb does. Visually hidden rather than
            invented chrome: the handle is already on screen in the chrome bar,
            it simply is not a heading there. */}
        <h1 className="sr-only">@{handle}</h1>
        {/* The lens's headline when one is open, the reader's lifetime anchor
            otherwise. The anchor is the only range-INDEPENDENT figure here;
            a lens core follows the range control like everything beside it. */}
        {lensCore ? (
          <CoreReadout
            value={lensCore.value}
            unit={lensCore.unit}
            label={lensCore.label}
            detail={lensCore.sub}
          />
        ) : (
          <CoreReadout
            value={lifetime.playtime}
            label="Logged flight time"
            detail={`${lifetime.events} events · ${lifetime.locations} places`}
          />
        )}
      </Depth>

      <Depth depth={54}>
        <CalloutField onOverflowClick={() => setEditing(true)}>
          {shownCallouts.map((c, i) => {
            const slot = slotFor(i);
            const body = (
              <Callout
                key={c.id}
                label={c.label}
                value={c.value}
                unit={c.unit}
                sub={c.sub}
                tone={c.tone}
                side={slot?.side ?? 'l'}
                at={slot?.at}
                onRemove={editing ? () => layout.remove(c.id) : undefined}
              />
            );
            return c.note ? (
              <BeamTip key={c.id} note={c.note} label={`How ${c.label} is derived`}>
                {body}
              </BeamTip>
            ) : (
              body
            );
          })}
        </CalloutField>

        {activeLabel ? (
          <Pane pane="detail" title={activeLabel}>
            {lensPlanes.length > 0 ? (
              lensPlanes.map((p) => (
                <React.Fragment key={p.id}>{p.node}</React.Fragment>
              ))
            ) : (
              <Flatline
                title={`Nothing under ${activeLabel} in this window`}
                reason="no-data"
              />
            )}
            {/* The trace `Holotable.jsx` puts under every lens, and which this
                screen shipped without.

                RANGE-INDEPENDENT and honest about it: the caption names its own
                window, because the panes above follow the range control and a
                chart that silently did not would be read as if it had. Real
                summed counts per calendar day — and no chart at all when there
                is no series, rather than a flat line that reads as "no
                activity" when it means "no data". */}
            {traceValues.length > 0 ? (
              <Trace
                cap={`Activity · last ${Math.round(traceDays / 7)} weeks`}
                mode="wave"
                values={traceValues}
              />
            ) : null}
          </Pane>
        ) : null}

        {record ? (
          <Pane pane="inspect" title={record}>
            {/* Degraded on purpose. The kit's inspector shows dwell, share,
                arrivals and deaths per place; the product has none of those —
                LocationsStatsResponse carries only unique_locations and capped
                visit counts. Rather than invent four figures, this states what
                it can and says what it cannot. The per-location aggregate is
                the backend follow-up that fills it in. */}
            <SubStats items={inspectStats(record, ringMap)} />
            <p className="hp-hint" style={{ marginTop: 14 }}>
              Dwell time and arrivals aren’t available yet — they need a
              per-location aggregate the API doesn’t serve.
            </p>
          </Pane>
        ) : null}
      </Depth>
      {/* Brand book §11: the attribution and outbound links must be reachable
          from every signed-in surface. The flat `.ss-app-footer` carried that
          obligation and `projection-shell.css` hides it, so the plate is here.
          Below the volume, not inside it — the ring owns the stage. */}
      <div className="hp-me-legal">
        <SiteLegalPlate />
      </div>
    </Projection>
    </div>
  );
}

/**
 * What the inspector can honestly say today: the stop's visit weight and which
 * system it sits in, both already on the ring. Missing is `—`, never `0`.
 */
function inspectStats(name: string, map: MapLayout): SubStatItem[] {
  const node = map.nodes.find((n) => n.n === name);
  const links = map.links.filter(
    (l) => map.nodes[l[0]]?.n === name || map.nodes[l[1]]?.n === name,
  );
  return [
    { k: 'System', v: node?.ctx ?? '—' },
    { k: 'Corridors', v: links.length > 0 ? String(links.length) : '—' },
    { k: 'Dwell', v: '—' },
    { k: 'Arrivals', v: '—' },
  ];
}
