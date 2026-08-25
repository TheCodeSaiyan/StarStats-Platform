import type { WidgetId } from '@/app/_components/widgets/types';

/**
 * The projection's element catalogue for `/me`.
 *
 * ELEMENT IDS ARE WIDGET IDS, deliberately. The reader's layout is already
 * persisted on the account as `LayoutEntry[]` keyed by widget id on the `home`
 * surface, and `LayoutSurface` is a server-side enum of exactly
 * `"profile" | "home"`. Minting a separate `co.*` / `lens.*` vocabulary would
 * have meant either a backend change (a third surface) or silently discarding
 * every existing reader's saved layout — layout ids are persisted, so renaming
 * one drops the layout that references it. Reusing the widget ids means an
 * owner who had enabled `spend` still has it on after the port, and
 * `saveProfileLayoutAction` works untouched.
 *
 * What IS new is how an element is drawn. Gap A2 ("split by shape"):
 *
 *   - `kind: 'callout'` — a single figure plus one line of arithmetic. Hangs in
 *     the volume on a leader line. Six slots, three a side.
 *   - `kind: 'plane'`   — a compound readout (a ranked list, a breakdown, a
 *     grid). Lives inside its lens's Pane as a `Plane`.
 *   - `kind: 'ring'`    — drawn BY the ring itself rather than beside it.
 *
 * A widget can appear twice: `economy` and `sessions` each contribute a callout
 * headline AND a pane, because they were compound tiles in the flat system.
 * That is one element id in two places, not two elements — the reader turns the
 * widget off and both go.
 */
export type ElementKind = 'callout' | 'plane' | 'ring';

export interface ProjectionElement {
  /** PERSISTED — this is a `WidgetId`. Never rename one. */
  id: WidgetId;
  /** Editor label. */
  name: string;
  /** Editor grouping. */
  group: 'Callouts' | 'Lens panes' | 'Centre ring';
  kind: ElementKind;
  /** Clarifies an ambiguous name in the editor. */
  hint?: string;
}

/**
 * Every element the /me projection can draw.
 *
 * LENS MEMBERSHIP IS NOT DECLARED HERE. It lives in `lib/lens.ts`
 * (`WIDGET_LENSES` / `widgetMatchesLens`), which is exhaustive on `WidgetId` by
 * type precisely because entries were missed seven times out of twenty-one.
 * This file duplicated it at first and immediately drifted: `facts` was given
 * `lens: null` meaning "All only", the rail had no All, and a widget enabled by
 * default therefore rendered nowhere at all. One map, and it is that one.
 */
export const PROJECTION_CATALOGUE: readonly ProjectionElement[] = [
  // ── Callouts: one figure, one supporting line ───────────────────────────
  {
    id: 'lives',
    name: 'Longest life',
    group: 'Callouts',
    kind: 'callout',
  },
  {
    id: 'contracts',
    name: 'Contracts',
    group: 'Callouts',
    kind: 'callout',
  },
  {
    id: 'objectives',
    name: 'Objectives',
    group: 'Callouts',
    kind: 'callout',
  },
  {
    id: 'spend',
    name: 'Spending',
    group: 'Callouts',
    kind: 'callout',
  },
  {
    id: 'economy',
    name: 'Orders',
    group: 'Callouts',
    kind: 'callout',
    hint: 'Commerce headline — the breakdown shows under the Commerce lens',
  },
  {
    id: 'sessions',
    name: 'Play sessions',
    group: 'Callouts',
    kind: 'callout',
  },
  {
    id: 'travel',
    name: 'Quantum transits',
    group: 'Callouts',
    kind: 'callout',
  },

  // ── Lens panes: compound readouts ───────────────────────────────────────
  {
    id: 'combat_mission',
    name: 'Combat & contracts',
    group: 'Lens panes',
    kind: 'plane',
    // NOT a callout, though it has a headline figure: the lifetime K/D lives in
    // the chrome as a range-independent identity readout, so a second K/D in
    // the range-scoped callout field would put two different numbers under one
    // name. The pane carries the window's breakdown instead.
  },
  {
    id: 'heatmap',
    name: 'Activity shape',
    group: 'Lens panes',
    kind: 'plane',
  },
  {
    id: 'routes',
    name: 'Top routes',
    group: 'Lens panes',
    kind: 'plane',
  },
  {
    id: 'corridors',
    name: 'Top corridors',
    group: 'Lens panes',
    kind: 'plane',
  },
  {
    id: 'fleet',
    name: 'Ships you fly',
    group: 'Lens panes',
    kind: 'plane',
  },
  {
    id: 'docking',
    name: 'Where you dock',
    group: 'Lens panes',
    kind: 'plane',
  },
  {
    id: 'locations',
    name: 'Places visited',
    group: 'Lens panes',
    kind: 'plane',
  },
  {
    id: 'facts',
    name: 'Flight facts',
    group: 'Lens panes',
    kind: 'plane',
  },

  // ── Centre ring ────────────────────────────────────────────────────────────────
  {
    id: 'journey',
    name: 'Route map',
    // "Ring" alone did not say WHICH ring, and a reader asked whether it
    // meant the one in the middle of the screen. It does — this element is
    // drawn BY that ring rather than beside it, which is the whole reason it
    // is not a pane or a callout, so the group says where to look.
    group: 'Centre ring',
    kind: 'ring',
    hint: 'Turns the ring at the centre of the volume into a route map, under the Travel lens',
  },

  // ── Restored after the first pass dropped them ──────────────────────────
  // These six are in the widget registry and were enable-able on the flat
  // dashboard, so a reader who had turned any of them on would have lost it
  // silently. Lens membership comes from `WIDGET_LENSES`; `records`, `orgs`
  // and `entities` belong to no single dimension and therefore show under All.
  {
    id: 'recent_activity',
    name: 'Recent activity',
    group: 'Lens panes',
    kind: 'plane',
  },
  {
    id: 'records',
    name: 'Records',
    group: 'Lens panes',
    kind: 'plane',
  },
  {
    id: 'stability',
    name: 'Stability',
    group: 'Lens panes',
    kind: 'plane',
  },
  {
    id: 'orgs',
    name: 'Orgs',
    group: 'Lens panes',
    kind: 'plane',
  },
  {
    id: 'hangar',
    name: 'Hangar',
    group: 'Lens panes',
    kind: 'plane',
    hint: 'Ships in your RSI hangar — written by the tray',
  },
  {
    id: 'loadout',
    name: 'Player loadout',
    group: 'Lens panes',
    kind: 'plane',
    hint: 'Last restored in-game kit — not the projection layout',
  },
  {
    id: 'entities',
    name: 'Entities rollup',
    group: 'Lens panes',
    kind: 'plane',
  },
];

export const ELEMENTS_BY_ID = new Map(
  PROJECTION_CATALOGUE.map((e) => [e.id, e] as const),
);

/** Every widget id the projection knows how to draw. */
export const PROJECTION_IDS: readonly WidgetId[] = PROJECTION_CATALOGUE.map(
  (e) => e.id,
);

/** Catalogue entries in the shape the `LayoutEditor` wants. */
export function editorCatalogue(enabledIds: readonly string[]) {
  return PROJECTION_CATALOGUE.map((e) => ({
    id: e.id,
    name: e.name,
    group: e.group,
    hint: e.hint,
    on: enabledIds.includes(e.id),
  }));
}
