/**
 * Client-safe widget metadata: title, one-line palette description, and
 * the per-widget size envelope (min-viable → max ceiling, in grid cells).
 *
 * This is the SINGLE source of truth for those three facts. It lives in a
 * plain data module (no `server-only`, no async) — exactly like
 * `tile-spans.ts` — because the pieces that need it run on the CLIENT and
 * therefore cannot import the server-only `WidgetDef`s:
 *   - `SortableProfileWidgets` renders the drag/resize grid + the
 *     "Add widget" palette (needs title + description for every widget,
 *     including ones not currently on the grid),
 *   - the resize/drag maths clamp to each widget's `bounds`,
 *   - `WidgetCanvas.titleFor` (server) reads the same titles so there's
 *     no second copy to drift.
 *
 * The `Record<WidgetId, WidgetMeta>` type is load-bearing: adding a new
 * `WidgetId` fails the build until it gets an entry here, which keeps the
 * palette, titles, and bounds exhaustive.
 */
import type { GridBounds } from './grid-layout';
import type { WidgetId } from './types';

export interface WidgetMeta {
  /** Tile title (also the palette entry heading). */
  title: string;
  /** Short palette description — what the widget shows, one line. */
  description: string;
  /** Size envelope in grid cells: the tile can't shrink below
   *  `minW × minH` (always shows its core datum) nor grow past
   *  `maxW × maxH` (so it can't swallow the dashboard). */
  bounds: GridBounds;
}

/**
 * Per-widget metadata. Bounds are in 24-col × 22px-row units. `minW/minH`
 * are picked so the tile always fits its min-viable readout; `maxW/maxH`
 * cap growth to what the content can meaningfully fill. Single-readout
 * widgets get tight ceilings; content-dense widgets (heatmap, journey,
 * recent activity) get tall ones.
 */
export const WIDGET_META: Record<WidgetId, WidgetMeta> = {
  sessions: {
    title: 'Play sessions',
    description: 'Session count, hours and cadence.',
    bounds: { minW: 8, minH: 5, maxW: 24, maxH: 12 },
  },
  heatmap: {
    title: 'Daily activity',
    description: 'Calendar heatmap of your play days.',
    bounds: { minW: 12, minH: 6, maxW: 24, maxH: 12 },
  },
  orgs: {
    title: 'Orgs',
    description: 'Your RSI organizations.',
    bounds: { minW: 5, minH: 4, maxW: 14, maxH: 10 },
  },
  entities: {
    title: 'Entities rollup',
    description: 'Ships, weapons and items you interact with.',
    bounds: { minW: 5, minH: 3, maxW: 16, maxH: 6 },
  },
  combat_mission: {
    title: 'Combat & Missions',
    description: 'Kills, deaths and mission outcomes.',
    bounds: { minW: 7, minH: 5, maxW: 24, maxH: 12 },
  },
  economy: {
    title: 'Economy',
    description: 'Buys, sells and spend in the window.',
    bounds: { minW: 6, minH: 3, maxW: 16, maxH: 6 },
  },
  travel: {
    title: 'Travel',
    description: 'Quantum jumps and distance covered.',
    bounds: { minW: 8, minH: 4, maxW: 24, maxH: 16 },
  },
  journey: {
    title: 'Journey',
    description: 'Route map and travel timeline.',
    bounds: { minW: 10, minH: 4, maxW: 24, maxH: 14 },
  },
  records: {
    title: 'Records',
    description: 'Personal bests and milestones.',
    bounds: { minW: 6, minH: 3, maxW: 16, maxH: 6 },
  },
  recent_activity: {
    title: 'Recent activity',
    description: 'Your latest tracked events.',
    bounds: { minW: 8, minH: 6, maxW: 24, maxH: 14 },
  },
  hangar: {
    title: 'Hangar',
    description: 'Ships in your RSI hangar.',
    bounds: { minW: 6, minH: 5, maxW: 16, maxH: 10 },
  },
  loadout: {
    title: 'Loadout',
    description: 'Your last equipped gear.',
    bounds: { minW: 6, minH: 4, maxW: 16, maxH: 7 },
  },
  lives: {
    title: 'Lives',
    description: 'Character survival stats.',
    bounds: { minW: 6, minH: 3, maxW: 16, maxH: 6 },
  },
  fleet: {
    title: 'Ships you fly',
    description: 'Ships ranked by quantum travel.',
    bounds: { minW: 6, minH: 5, maxW: 14, maxH: 9 },
  },
  docking: {
    title: 'Where you dock',
    description: 'Hangar-vs-pad split and ship sizes.',
    bounds: { minW: 8, minH: 5, maxW: 16, maxH: 10 },
  },
  objectives: {
    title: 'Mission objectives',
    description: 'Objective completion rate and outcomes.',
    bounds: { minW: 5, minH: 3, maxW: 14, maxH: 6 },
  },
  contracts: {
    title: 'Contracts',
    description: 'Contract-run outcomes and completion rate.',
    bounds: { minW: 5, minH: 3, maxW: 14, maxH: 6 },
  },
  spend: {
    title: 'Spending',
    description: 'aUEC spent, purchases and top shop.',
    bounds: { minW: 5, minH: 3, maxW: 14, maxH: 6 },
  },
  routes: {
    title: 'Top routes',
    description: 'Most-travelled quantum destinations.',
    bounds: { minW: 6, minH: 5, maxW: 16, maxH: 9 },
  },
  locations: {
    title: 'Places visited',
    description: 'Distinct locations you have visited.',
    bounds: { minW: 6, minH: 5, maxW: 16, maxH: 9 },
  },
  facts: {
    title: 'Flight facts',
    description: 'Observations about how you fly, with the numbers behind them.',
    bounds: { minW: 6, minH: 5, maxW: 16, maxH: 12 },
  },
  corridors: {
    title: 'Top corridors',
    description: 'Busiest travel legs between two stops.',
    bounds: { minW: 6, minH: 5, maxW: 16, maxH: 9 },
  },
};

/** Resolve a widget's size envelope, defaulting gracefully for an id that
 *  somehow isn't in the map (never expected — the Record type enforces
 *  completeness — but keeps the grid maths total). */
export function boundsForWidget(id: string): GridBounds {
  return (
    WIDGET_META[id as WidgetId]?.bounds ?? {
      minW: 5,
      minH: 4,
      maxW: 24,
      maxH: 24,
    }
  );
}

/** Title lookup used by both the server canvas and the client palette. */
export function titleForWidget(id: WidgetId): string {
  return WIDGET_META[id].title;
}
