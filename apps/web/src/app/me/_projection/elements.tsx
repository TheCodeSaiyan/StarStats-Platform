import 'server-only';

import React from 'react';
import type { Route } from 'next';
import Link from 'next/link';
import { Plane, MeterRow, LogRow } from 'holo';
import { WIDGETS_BY_ID } from '@/app/_components/widgets/registry';
import type { ViewerCtx, WidgetId } from '@/app/_components/widgets/types';
import { logger } from '@/lib/logger';
import { fmtDuration, fmtNum, fmtPct } from '@/app/_components/widgets/kit/format';
import { EntityLink } from '@/components/kb/EntityLink';
import type { DockingResponse, StatsBucket } from '@/lib/api';
import { loadAllReferenceBundles } from '@/lib/reference';
import type { ReferenceCatalog, ReferenceCatalogs } from '@/lib/reference-types';

/** The catalogues the ranked planes resolve their raw identifiers against. */
type Catalogs = ReferenceCatalogs;

/**
 * Projection bodies for the /me elements.
 *
 * These reuse each widget's `load()` — the exact same endpoint calls, empty
 * checks, trend maths and provenance caveats the flat dashboard uses — and draw
 * the result in the holographic language instead. Nothing here fetches; nothing
 * in the widget layer changed except that `load` became reachable.
 *
 * Two rules from the design system govern every formatter below:
 *   - Missing is `—`, NEVER `0` and never blank. A zero means "we looked and
 *     there were none", which reads very differently from "we have nothing".
 *   - State the limit. An inferred figure carries its derivation rather than
 *     rounding the caveat away.
 */

/** A callout is serialisable data, not a node: the client assigns it a slot
 *  from the reader's layout order, so it cannot be pre-rendered server-side. */
export interface CalloutVM {
  id: WidgetId;
  /**
   * The widget's OTHER figures, for the lens pane.
   *
   * A callout is one figure by design — it hangs in the volume on a leader
   * line and there is room for a value and a line of arithmetic. The widget
   * behind it usually holds three to five: `lives` knows total lives, deaths
   * and mean life; `combat_mission` knows deaths, vehicle losses and mission
   * completion. None of that had anywhere to go in the projection, so opening
   * a lens showed strictly less than the flat widget it replaced.
   *
   * The lens pane renders these as its `SubStats` row. Absent means "the
   * headline is all there is", and the pane falls back to the callout itself.
   */
  stats?: { k: string; v: string; u?: string; tone?: 'warn' | 'good' | 'bad' }[];
  label: string;
  value: string;
  unit?: string;
  sub?: string;
  tone?: 'warn' | 'good' | 'bad';
  /** Derivation for an inferred figure. Rendered as a `BeamTip`. */
  note?: string;
}

/** A pane element IS a node — it is rendered server-side and handed to the
 *  client component as a prop, which RSC supports. */
export interface PlaneVM {
  id: WidgetId;
  node: React.ReactNode;
}

/** The em-dash is the missing marker throughout. */
const MISSING = '—';

/**
 * The leading token of a bounded count label.
 *
 * `buildSessionSummary` produces "128 sessions" / "1 session" / "50+ sessions"
 * — the `+` matters, because the list endpoint is capped and the label is how
 * that cap stays visible. A callout wants the figure and the noun apart, so
 * this splits on the first space and keeps any `+` with the figure.
 */
function splitCount(label: string): { value: string; unit: string } {
  const i = label.indexOf(' ');
  if (i < 0) return { value: label, unit: '' };
  return { value: label.slice(0, i), unit: label.slice(i + 1) };
}

// ── Callout builders ──────────────────────────────────────────────────────
//
// Each takes the widget's own loaded shape and returns the figure plus its one
// line of arithmetic. Typed against the shapes declared in the widget files;
// `load` is typed `unknown` on `WidgetDef` because each widget's shape is its
// own, so each builder narrows to the one it knows.

interface LivesData {
  total_lives: number;
  deaths: number;
  deaths_inferred: number;
  longest_life_secs: number | null;
  mean_life_secs: number | null;
}

function livesCallout(d: LivesData): CalloutVM {
  return {
    id: 'lives',
    label: 'Longest life',
    value: d.longest_life_secs != null ? fmtDuration(d.longest_life_secs) : MISSING,
    sub:
      d.mean_life_secs != null
        ? `mean ${fmtDuration(d.mean_life_secs)}`
        : `${fmtNum(d.total_lives)} lives`,
    tone: 'good',
    stats: [
      { k: 'Longest life', v: d.longest_life_secs != null ? fmtDuration(d.longest_life_secs) : MISSING },
      { k: 'Mean life', v: d.mean_life_secs != null ? fmtDuration(d.mean_life_secs) : MISSING },
      { k: 'Lives', v: fmtNum(d.total_lives) },
      { k: 'Deaths', v: fmtNum(d.deaths), tone: 'warn' },
    ],
    // Deaths are partly reconstructed, and a life LENGTH is bounded by the
    // deaths that end it — so the caveat travels with this figure too.
    note:
      d.deaths_inferred > 0
        ? `${fmtNum(d.deaths_inferred)} of ${fmtNum(d.deaths)} deaths were reconstructed from Corpse lines, as the game no longer logs deaths directly. Lives are bounded by those deaths.`
        : undefined,
  };
}

interface ContractsData {
  completed: number;
  resolved: number;
  pct: number | null;
}

function contractsCallout(d: ContractsData): CalloutVM {
  return {
    id: 'contracts',
    stats: [
      { k: 'Completed', v: fmtNum(d.completed) },
      { k: 'Resolved', v: fmtNum(d.resolved) },
      { k: 'Rate', v: d.resolved > 0 && d.pct != null ? fmtPct(d.pct, true) : MISSING },
    ],
    label: 'Contracts',
    value: fmtNum(d.completed),
    // The denominator is RESOLVED, not started — an in-progress contract has
    // no outcome yet, and counting it as a miss would be materially misleading.
    sub:
      d.pct != null
        ? `${fmtPct(d.pct, true)} of ${fmtNum(d.resolved)} resolved`
        : 'none resolved yet',
  };
}

interface ObjectivesData {
  completed: number;
  resolved: number;
  pct: number;
}

function objectivesCallout(d: ObjectivesData): CalloutVM {
  return {
    id: 'objectives',
    stats: [
      { k: 'Completion', v: d.resolved > 0 && d.pct != null ? fmtPct(d.pct, true) : MISSING },
      { k: 'Completed', v: fmtNum(d.completed) },
      { k: 'Resolved', v: fmtNum(d.resolved) },
    ],
    label: 'Objectives',
    value: d.resolved > 0 ? fmtPct(d.pct, true) : MISSING,
    sub:
      d.resolved > 0
        ? `${fmtNum(d.completed)} of ${fmtNum(d.resolved)} resolved`
        : 'nothing resolved in this window',
  };
}

interface SpendData {
  total_auec: number;
  purchases: number;
  top_shop: string | null;
}

function spendCallout(d: SpendData): CalloutVM {
  return {
    id: 'spend',
    stats: [
      { k: 'Spent', v: fmtNum(d.total_auec), u: 'aUEC' },
      { k: 'Purchases', v: fmtNum(d.purchases) },
      ...(d.top_shop ? [{ k: 'Top shop', v: d.top_shop }] : []),
    ],
    label: 'Spending',
    value: fmtNum(d.total_auec),
    unit: 'aUEC',
    sub: d.top_shop
      ? `${fmtNum(d.purchases)} purchases · ${d.top_shop}`
      : `${fmtNum(d.purchases)} purchases`,
    tone: 'warn',
  };
}

interface EconomyData {
  buys: number;
  sells: number;
  pending: number;
}

function economyCallout(d: EconomyData): CalloutVM {
  return {
    id: 'economy',
    stats: [
      { k: 'Orders', v: fmtNum(d.buys + d.sells) },
      { k: 'Buys', v: fmtNum(d.buys) },
      { k: 'Sells', v: fmtNum(d.sells) },
      ...(d.pending > 0 ? [{ k: 'Pending', v: fmtNum(d.pending), tone: 'warn' as const }] : []),
    ],
    label: 'Orders',
    value: fmtNum(d.buys + d.sells),
    sub: `${fmtNum(d.buys)} buys · ${fmtNum(d.sells)} sells`,
  };
}

interface SessionsData {
  summary: { countLabel: string; totalHoursLabel: string | null };
}

function sessionsCallout(d: SessionsData): CalloutVM {
  const { value, unit } = splitCount(d.summary.countLabel);
  return {
    id: 'sessions',
    label: 'Play sessions',
    value,
    unit,
    sub: d.summary.totalHoursLabel ?? undefined,
  };
}

interface TravelData {
  quantums: number;
  serverHops: number;
  planets: number;
}

function travelCallout(d: TravelData): CalloutVM {
  return {
    id: 'travel',
    stats: [
      { k: 'Quantum', v: fmtNum(d.quantums) },
      { k: 'Server hops', v: fmtNum(d.serverHops) },
      { k: 'Planets', v: fmtNum(d.planets) },
    ],
    label: 'Quantum transits',
    value: fmtNum(d.quantums),
    unit: 'jumps',
    sub: `${fmtNum(d.serverHops)} server hops`,
    // Quantum jumps are INFERRED from the log rather than logged as such. The
    // system's rule is that an inferred metric carries its derivation.
    note: 'Inferred from quantum-travel log lines — the game does not log a jump count directly.',
  };
}

// ── Plane builders ────────────────────────────────────────────────────────

interface RankedRow {
  name: React.ReactNode;
  value: string;
  pct: number;
}

/**
 * A row built from a catalogued entity.
 *
 * Two faults this fixes at once, and they share a cause — the planes were
 * rendering the RAW value the API returns:
 *
 *   - The name was an engine identifier. "Ships you fly" listed
 *     `AEGS_Avenger_Stalker`, "Places visited" and "Where you dock" listed raw
 *     location keys. The catalogue that resolves those has been loaded on this
 *     page all along, for the hover cards.
 *   - The row went nowhere. `MeterRow` takes an `onClick` and this module never
 *     passed one — it CANNOT, being a server module, since a handler does not
 *     cross the RSC boundary. `rankedPlane`'s own docstring claimed "rows open
 *     the in-volume inspector"; nothing was ever wired.
 *
 * `EntityLink` answers both: it resolves the display name from the catalogue
 * and renders a real `<Link>` to `/kb/{category}/{slug}`. A link is also the
 * better answer than a click handler — it is shareable, it survives the back
 * button, and it works before hydration.
 *
 * No catalogue match means no link and no rewrite: `EntityLink` falls through
 * to plain text, so a row is never worse off than it was.
 */
function entityRow(
  category: 'vehicle' | 'location' | 'weapon' | 'item',
  classKey: string,
  catalog: ReferenceCatalog | undefined,
  value: string,
  pct: number,
): RankedRow {
  return {
    name: (
      <EntityLink category={category} classKey={classKey} catalog={catalog} />
    ),
    value,
    pct,
  };
}

/**
 * The ranked Plane — the shape `RankedList` had in the flat kit.
 *
 * `trailing` is the real route out (gap A7): rows open the in-volume inspector,
 * and the Plane's caption carries a Next <Link> to the full page, so the depth
 * model stays intact without losing a crawlable URL.
 */
function rankedPlane(
  cap: string,
  rows: RankedRow[],
  opts: { href?: Route; hint?: string; onSelectable?: boolean } = {},
): React.ReactNode {
  return (
    <Plane
      cap={cap}
      hint={opts.hint}
      trailing={
        opts.href ? <Link href={opts.href}>see all →</Link> : undefined
      }
      empty={<span className="hp-empty">{MISSING} nothing in this window</span>}
    >
      {rows.map((r, i) => (
        <MeterRow
          key={i}
          rank={i + 1}
          name={r.name}
          value={r.value}
          pct={r.pct}
        />
      ))}
    </Plane>
  );
}

/** Share of the largest row, which is what the meter bar encodes. */
function pctOf(count: number, max: number): number {
  return max > 0 ? (count / max) * 100 : 0;
}

interface RoutesData {
  routes: ReadonlyArray<{ destination: string; count: number }>;
}

function routesPlane(d: RoutesData, cat?: Catalogs): React.ReactNode {
  const max = Math.max(...d.routes.map((r) => r.count), 0);
  return rankedPlane(
    'Top routes',
    d.routes
      .slice(0, 6)
      .map((r) =>
        entityRow(
          'location',
          r.destination,
          cat?.locations,
          fmtNum(r.count),
          pctOf(r.count, max),
        ),
      ),
    { href: '/me/travel' as Route, hint: 'select a destination →' },
  );
}

interface CorridorsData {
  corridors: ReadonlyArray<{ a: string; b: string; count: number }>;
  maxCount: number;
  soleStop: string | null;
}

function corridorsPlane(d: CorridorsData): React.ReactNode {
  if (d.corridors.length === 0 && d.soleStop) {
    // A fact, not an absence: the player had telemetry but never left.
    return (
      <Plane cap="Top corridors">
        <span className="hp-empty">
          No travel between stops — every event at {d.soleStop}.
        </span>
      </Plane>
    );
  }
  return rankedPlane(
    'Top corridors',
    d.corridors.slice(0, 6).map((c) => ({
      name: `${c.a} ⇄ ${c.b}`,
      value: fmtNum(c.count),
      pct: pctOf(c.count, d.maxCount),
    })),
    { href: '/me/travel' as Route },
  );
}

interface FleetData {
  ships: ReadonlyArray<{ vehicle_class: string; trip_count: number }>;
}

function fleetPlane(d: FleetData, cat?: Catalogs): React.ReactNode {
  const max = Math.max(...d.ships.map((s) => s.trip_count), 0);
  return rankedPlane(
    'Ships you fly',
    d.ships
      .slice(0, 6)
      .map((s) =>
        entityRow(
          'vehicle',
          s.vehicle_class,
          cat?.vehicles,
          fmtNum(s.trip_count),
          pctOf(s.trip_count, max),
        ),
      ),
    { href: '/kb/vehicle' as Route, hint: 'select a ship →' },
  );
}

/**
 * THE REAL SHAPE, taken from the widget rather than assumed.
 *
 * This was declared as `by_kind: ReadonlyArray<{key, count}>` and the endpoint
 * returns an OBJECT — `{ hangar, pad, other }`. So the plane threw
 * `by_kind.map is not a function` on every real render, and had done since it
 * was written: `/me` logged `projection element failed` and dropped the plane
 * silently, because a failed element is caught per-element by design.
 *
 * TypeScript could not see it. `BUILDERS` casts every builder through
 * `as (d: never) => React.ReactNode`, which erases the relationship between
 * what a widget LOADS and what its builder CLAIMS to receive — so a local
 * interface that disagrees with the API compiles cleanly and fails only in
 * production. Sourcing the type from `DockingResponse` is what makes the
 * compiler check it.
 */
interface DockingData {
  by_kind: DockingResponse['by_kind'];
  total: number;
}

function dockingPlane(d: DockingData): React.ReactNode {
  // Kinds of berth, not places — so these rows are labels, not catalogued
  // entities, and they do not link anywhere.
  const kinds: { name: string; count: number }[] = [
    { name: 'Hangar', count: d.by_kind.hangar },
    { name: 'Pad', count: d.by_kind.pad },
    { name: 'Other', count: d.by_kind.other },
  ].filter((k) => k.count > 0);
  const max = Math.max(...kinds.map((k) => k.count), 0);
  return rankedPlane(
    'Where you dock',
    kinds.map((k) => ({
      name: k.name,
      value: fmtNum(k.count),
      pct: pctOf(k.count, max),
    })),
    { hint: `${fmtNum(d.total)} total` },
  );
}

/**
 * SILENT VERSION OF THE SAME FAULT as `DockingData`.
 *
 * This declared `top_locations?` and the widget returns `top` — so the optional
 * chain resolved to `undefined`, the rows were `[]`, and "Places visited"
 * rendered as an empty plane on every account with locations. No crash, no log
 * entry: it looked exactly like a reader who had been nowhere.
 *
 * The widget also hands back the catalogue it already loaded, so the names
 * resolve from that rather than from a second load in this module.
 */
interface LocationsData {
  top: ReadonlyArray<StatsBucket>;
  unique: number;
  locations?: ReferenceCatalog;
}

function locationsPlane(d: LocationsData, cat?: Catalogs): React.ReactNode {
  const rows = d.top ?? [];
  const max = Math.max(...rows.map((r) => r.count), 0);
  return rankedPlane(
    'Places visited',
    rows
      .slice(0, 6)
      .map((r) =>
        entityRow(
          'location',
          r.value,
          d.locations ?? cat?.locations,
          fmtNum(r.count),
          pctOf(r.count, max),
        ),
      ),
    { href: '/me/travel' as Route, hint: 'select a place →' },
  );
}

interface CombatMissionData {
  deaths: number;
  vehicleLosses: number;
  missionsStarted: number;
  missionsEnded: number;
  completionPct: number | null;
}

function combatPlane(d: CombatMissionData): React.ReactNode {
  const rows: RankedRow[] = [
    { name: 'Contracts started', value: fmtNum(d.missionsStarted), pct: 0 },
    { name: 'Contracts ended', value: fmtNum(d.missionsEnded), pct: 0 },
    { name: 'Deaths', value: fmtNum(d.deaths), pct: 0 },
    { name: 'Hulls lost', value: fmtNum(d.vehicleLosses), pct: 0 },
  ];
  const max = Math.max(
    d.missionsStarted,
    d.missionsEnded,
    d.deaths,
    d.vehicleLosses,
    0,
  );
  return (
    <Plane
      cap="Combat & contracts"
      hint={
        d.completionPct != null
          ? `${fmtPct(d.completionPct, true)} completed`
          : undefined
      }
      trailing={<Link href={'/me/contracts' as Route}>see all →</Link>}
      empty={<span className="hp-empty">{MISSING} nothing in this window</span>}
    >
      {rows.map((r, i) => (
        <MeterRow
          key={i}
          rank={i + 1}
          name={r.name}
          value={r.value}
          pct={pctOf(Number(r.value.replace(/,/g, '')), max)}
        />
      ))}
    </Plane>
  );
}

interface PlayerFact {
  id: string;
  headline: string;
  detail: string;
}

interface FactsData {
  facts: readonly PlayerFact[];
  enoughHistory: boolean;
  sessionsConsidered: number;
  sessionsRequired: number;
}

/**
 * Flight facts — one of the three widgets with no archetype behind it, so it
 * gets a purpose-built flat Plane rather than being forced into a ranked list.
 * Each observation shows its own arithmetic beneath it, which is the whole
 * point of the surface: the claim shows its working.
 */
function factsPlane(d: FactsData): React.ReactNode {
  if (!d.enoughHistory) {
    return (
      <Plane tilt="flat" cap="Flight facts">
        <span className="hp-empty">
          {d.sessionsConsidered} of {d.sessionsRequired} sessions — not enough
          history for an observation to mean anything yet.
        </span>
      </Plane>
    );
  }
  return (
    <Plane tilt="flat" cap="Flight facts">
      {d.facts.map((f) => (
        <div key={f.id} className="hp-fact">
          <span className="hp-fact__h">{f.headline}</span>
          <span className="hp-fact__d">{f.detail}</span>
        </div>
      ))}
    </Plane>
  );
}

interface TimelineData {
  buckets: ReadonlyArray<{ date: string; count: number }>;
}

/**
 * Activity shape — the calendar heatmap, redrawn in beam alphas.
 *
 * Ranking is BRIGHTNESS, never hue: the system has one colour per calibration
 * and a categorical palette would break a calibration swap. Cells carry a
 * `title` so the exact figure is reachable without a legend.
 */
function heatmapPlane(d: TimelineData): React.ReactNode {
  const max = Math.max(...d.buckets.map((b) => b.count), 0);
  return (
    <Plane tilt="flat" cap="Activity shape" hint={`peak ${fmtNum(max)}/day`}>
      <div className="hp-cal-grid">
        {d.buckets.map((b) => {
          // Five steps of alpha. A day with zero events is drawn as an empty
          // cell rather than omitted, so the rhythm of gaps stays visible.
          const step = max > 0 ? Math.ceil((b.count / max) * 4) : 0;
          return (
            <i
              key={b.date}
              data-step={step}
              title={`${b.date} · ${fmtNum(b.count)} events`}
            />
          );
        })}
      </div>
    </Plane>
  );
}

interface OrgsData {
  orgs: ReadonlyArray<{ name: string; rank?: string | null; sid?: string | null }>;
  capturedAt: string;
}

function orgsPlane(d: OrgsData): React.ReactNode {
  return (
    <Plane
      tilt="flat"
      cap="Orgs"
      hint={`${d.orgs.length}`}
      empty={<span className="hp-empty">{MISSING} no orgs on the snapshot</span>}
    >
      {d.orgs.slice(0, 6).map((o, i) => (
        <MeterRow
          key={o.sid ?? o.name ?? i}
          rank={i + 1}
          name={o.name}
          value={o.rank ?? ''}
          valueText
        />
      ))}
    </Plane>
  );
}

interface RecentActivityData {
  events: ReadonlyArray<{
    seq?: number;
    event_type: string;
    event_timestamp?: string | null;
  }>;
}

function recentActivityPlane(d: RecentActivityData): React.ReactNode {
  return (
    <Plane
      tilt="flat"
      cap="Recent activity"
      hint="most recent first"
      trailing={<Link href={'/me/contracts' as Route}>see all →</Link>}
      empty={<span className="hp-empty">{MISSING} nothing in this window</span>}
    >
      {d.events.slice(0, 8).map((e, i) => (
        <LogRow
          key={e.seq ?? i}
          time={
            e.event_timestamp
              ? new Date(e.event_timestamp).toLocaleTimeString([], {
                  hour: '2-digit',
                  minute: '2-digit',
                })
              : MISSING
          }
          // The raw discriminant stays addressable rather than being
          // prettified into something the log never said.
          event={e.event_type}
        />
      ))}
    </Plane>
  );
}

interface RecordsData {
  longestSessionSecs: number;
  busiestSessionEvents: number;
  biggestTradeQuantity: number;
  biggestTradeItem: string | null;
  longestSurvivalStreakSecs: number;
}

function recordsPlane(d: RecordsData): React.ReactNode {
  const rows: RankedRow[] = [
    {
      name: 'Longest session',
      value: d.longestSessionSecs > 0 ? fmtDuration(d.longestSessionSecs) : MISSING,
      pct: 0,
    },
    {
      name: 'Busiest session',
      value: d.busiestSessionEvents > 0 ? fmtNum(d.busiestSessionEvents) : MISSING,
      pct: 0,
    },
    {
      name: d.biggestTradeItem ? `Biggest trade · ${d.biggestTradeItem}` : 'Biggest trade',
      value: d.biggestTradeQuantity > 0 ? fmtNum(d.biggestTradeQuantity) : MISSING,
      pct: 0,
    },
    {
      name: 'Longest stretch alive',
      value:
        d.longestSurvivalStreakSecs > 0
          ? fmtDuration(d.longestSurvivalStreakSecs)
          : MISSING,
      pct: 0,
    },
  ];
  return (
    <Plane tilt="flat" cap="Records" hint="personal bests">
      {rows.map((r, i) => (
        <MeterRow key={i} rank={i + 1} name={r.name} value={r.value} valueText />
      ))}
    </Plane>
  );
}

interface HangarData {
  ships: ReadonlyArray<{ name: string; manufacturer?: string | null; kind?: string | null }>;
}

function hangarPlane(d: HangarData): React.ReactNode {
  return (
    <Plane
      tilt="flat"
      cap="Hangar"
      hint={`${d.ships.length}`}
      // The server holds ZERO RSI credentials — only the tray scrapes the
      // pledges page with the reader's own cookie. So the affordance points at
      // the tray's surface, never at a web-side refresh that cannot exist.
      trailing={<Link href={'/downloads' as Route}>via tray →</Link>}
      empty={<span className="hp-empty">{MISSING} no hangar snapshot yet</span>}
    >
      {d.ships.slice(0, 8).map((sh, i) => (
        <MeterRow
          key={`${sh.name}-${i}`}
          rank={i + 1}
          name={sh.name}
          value={sh.manufacturer ?? sh.kind ?? ''}
          valueText
        />
      ))}
    </Plane>
  );
}

interface LoadoutPreviewItem {
  label?: string;
  name?: string;
  count?: number;
}
interface LoadoutViewData {
  count: number;
  preview: ReadonlyArray<LoadoutPreviewItem>;
  hasMoreClasses: boolean;
}

function loadoutPlane(d: LoadoutViewData): React.ReactNode {
  return (
    <Plane
      tilt="flat"
      cap="Player loadout"
      // "Loadout" here is the IN-GAME kit, never the projection layout. The
      // game owns that word; the two must never be swapped.
      hint={d.hasMoreClasses ? 'partial' : undefined}
      trailing={<Link href={'/me/loadout' as Route}>see all →</Link>}
      empty={<span className="hp-empty">{MISSING} no restore recorded</span>}
    >
      {d.preview.slice(0, 8).map((it, i) => (
        <MeterRow
          key={i}
          rank={i + 1}
          name={it.label ?? it.name ?? MISSING}
          value={it.count != null ? fmtNum(it.count) : ''}
          valueText
        />
      ))}
    </Plane>
  );
}

interface EntitiesData {
  counts?: Record<string, number>;
}

function entitiesPlane(d: EntitiesData): React.ReactNode {
  const entries = Object.entries(d.counts ?? {});
  return (
    <Plane
      tilt="flat"
      cap="Entities"
      trailing={<Link href={'/kb' as Route}>catalogue →</Link>}
      empty={<span className="hp-empty">{MISSING} nothing catalogued yet</span>}
    >
      {entries.map(([k, v], i) => (
        <MeterRow key={k} rank={i + 1} name={k} value={fmtNum(v)} valueText />
      ))}
    </Plane>
  );
}

// ── Dispatch ──────────────────────────────────────────────────────────────

type Builder =
  | { kind: 'callout'; build: (data: never) => CalloutVM }
  // Plane builders take the catalogues as a second argument so a row can
  // resolve a raw engine identifier to its catalogued name and link to it.
  // Optional, so the builders that have no entities in them ignore it.
  | { kind: 'plane'; build: (data: never, cat?: Catalogs) => React.ReactNode };

const BUILDERS: Partial<Record<WidgetId, Builder>> = {
  lives: { kind: 'callout', build: livesCallout as (d: never) => CalloutVM },
  contracts: { kind: 'callout', build: contractsCallout as (d: never) => CalloutVM },
  objectives: { kind: 'callout', build: objectivesCallout as (d: never) => CalloutVM },
  spend: { kind: 'callout', build: spendCallout as (d: never) => CalloutVM },
  economy: { kind: 'callout', build: economyCallout as (d: never) => CalloutVM },
  sessions: { kind: 'callout', build: sessionsCallout as (d: never) => CalloutVM },
  travel: { kind: 'callout', build: travelCallout as (d: never) => CalloutVM },
  routes: { kind: 'plane', build: routesPlane as (d: never) => React.ReactNode },
  corridors: { kind: 'plane', build: corridorsPlane as (d: never) => React.ReactNode },
  fleet: { kind: 'plane', build: fleetPlane as (d: never) => React.ReactNode },
  docking: { kind: 'plane', build: dockingPlane as (d: never) => React.ReactNode },
  locations: { kind: 'plane', build: locationsPlane as (d: never) => React.ReactNode },
  combat_mission: { kind: 'plane', build: combatPlane as (d: never) => React.ReactNode },
  facts: { kind: 'plane', build: factsPlane as (d: never) => React.ReactNode },
  heatmap: { kind: 'plane', build: heatmapPlane as (d: never) => React.ReactNode },
  orgs: { kind: 'plane', build: orgsPlane as (d: never) => React.ReactNode },
  recent_activity: {
    kind: 'plane',
    build: recentActivityPlane as (d: never) => React.ReactNode,
  },
  records: { kind: 'plane', build: recordsPlane as (d: never) => React.ReactNode },
  hangar: { kind: 'plane', build: hangarPlane as (d: never) => React.ReactNode },
  loadout: { kind: 'plane', build: loadoutPlane as (d: never) => React.ReactNode },
  entities: { kind: 'plane', build: entitiesPlane as (d: never) => React.ReactNode },
};

export interface BuiltElements {
  callouts: CalloutVM[];
  planes: PlaneVM[];
}

/**
 * Load and build every enabled element.
 *
 * `Promise.allSettled`, never `Promise.all`: one endpoint hiccup must degrade a
 * single element rather than blanking the whole projection, and each rejection
 * is logged with its `call=` label so the failing element is named in the
 * server logs. This is the same invariant the flat `WidgetCanvas` holds.
 */
export async function buildElements(
  ctx: ViewerCtx,
  enabledIds: readonly string[],
): Promise<BuiltElements> {
  const wanted = enabledIds.filter((id) => BUILDERS[id as WidgetId]);

  // The catalogues resolve raw identifiers into names and KB links. Built at
  // BUILD time from the static reference snapshot (`lib/reference.ts`), so this
  // is a memory read, not a fetch — and it is already loaded on this request by
  // the hover cards. Degrades to undefined, which `EntityLink` renders as plain
  // text: a row is never worse off than the raw value it showed before.
  let catalogs: Catalogs | undefined;
  try {
    catalogs = (await loadAllReferenceBundles()).catalogs;
  } catch (err) {
    logger.warn({ err, call: 'projection.catalogs' }, 'catalogue load failed');
  }

  const settled = await Promise.allSettled(
    wanted.map(async (id) => {
      const def = WIDGETS_BY_ID.get(id as WidgetId);
      if (!def?.load) return null;
      if (!(await def.isAvailable(ctx))) return null;
      const data = await def.load(ctx);
      // `null` is the widget contract for "no data / error" — not an error.
      if (data == null) return null;
      const builder = BUILDERS[id as WidgetId]!;
      if (builder.kind === 'callout') {
        return {
          kind: 'callout' as const,
          vm: (builder.build as (d: unknown) => CalloutVM)(data),
        };
      }
      return {
        kind: 'plane' as const,
        vm: {
          id: id as WidgetId,
          node: (
            builder.build as (d: unknown, c?: Catalogs) => React.ReactNode
          )(data, catalogs),
        },
      };
    }),
  );

  const callouts: CalloutVM[] = [];
  const planes: PlaneVM[] = [];
  settled.forEach((r, i) => {
    if (r.status === 'rejected') {
      logger.warn(
        { err: r.reason, call: `projection.element.${wanted[i]}` },
        'projection element failed',
      );
      return;
    }
    if (!r.value) return;
    if (r.value.kind === 'callout') callouts.push(r.value.vm);
    else planes.push(r.value.vm);
  });

  // Preserve the reader's layout order — slots fill in that order, so this is
  // what decides which six callouts get drawn and which report as overflow.
  const rank = new Map(enabledIds.map((id, i) => [id, i] as const));
  const byRank = <T extends { id: string }>(a: T, b: T) =>
    (rank.get(a.id) ?? 0) - (rank.get(b.id) ?? 0);
  callouts.sort(byRank);
  planes.sort(byRank);

  return { callouts, planes };
}

