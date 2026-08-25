/**
 * Free drag/resize grid geometry — pure logic (no React, no DOM).
 *
 * The widget dashboard used to be REORDER-ONLY: a 4-column CSS grid
 * (`hud-grid`) where each widget picked a column span and the array
 * order decided flow. Milestone M7 upgrades it to a free drag/resize
 * grid with **toggleable snapping**: each widget carries an explicit
 * position + size (`x, y, w, h`) on a fine 24-column grid.
 *
 * Backward compatibility is the load-bearing invariant here. Stored
 * layouts are `LayoutEntry[]` with only `{ id, enabled, size }` and NO
 * geometry. `ensureGeometry` derives a sensible free-grid position from
 * the legacy order + per-widget spans so a user who never customises
 * sees an unchanged dashboard, and a customised layout keeps every
 * position it already has (only newly-appended widgets get placed).
 *
 * Everything in this module is a pure function so the migration and the
 * snap/round-trip maths are unit-tested without rendering anything.
 */

import type { LayoutEntry } from '@/lib/api';
import type { WidgetId, WidgetSize } from './types';

/** Columns on the fine placement grid. 24 = 6× the old 4-col grid, so
 *  a legacy span-N tile maps cleanly to `N * 6` columns and users get
 *  "much higher positional anchor points" than the reorder grid. */
export const GRID_COLS = 24;

/** Height of one grid row, in px. Vertical resize/position works in
 *  these units. Kept small so vertical anchor points are dense too. */
export const ROW_PX = 22;

/** Minimum widget footprint so a tile never collapses to nothing. */
export const MIN_W = 3;
export const MIN_H = 3;

/** Absolute ceiling on tile height (rows). A generous cap so a widget
 *  can grow tall on a big dashboard, but never unbounded (which would
 *  let one tile push the whole grid off-screen). Per-widget `maxH` in
 *  {@link GridBounds} refines this downward. */
export const MAX_H = 40;

/**
 * Per-widget size envelope, in grid cells. A widget always shows its
 * min-viable datum at `minW × minH` and never grows past `maxW × maxH`,
 * so a dashboard tile can't be shrunk into uselessness or ballooned to
 * swallow the grid. Enforced by the resize + drag maths below and by the
 * palette placement. The authoritative per-widget values live in the
 * client-safe `widget-meta.ts` (`WIDGET_META`) — the grid is client code
 * and can't import the server-only `WidgetDef`s, the same reason
 * `TILE_SPANS` is a plain map.
 */
export interface GridBounds {
  minW: number;
  minH: number;
  maxW: number;
  maxH: number;
}

/** Fallback envelope when a widget declares no explicit bounds: the full
 *  grid width and the global height cap. */
export const DEFAULT_BOUNDS: GridBounds = {
  minW: MIN_W,
  minH: MIN_H,
  maxW: GRID_COLS,
  maxH: MAX_H,
};

/** The four geometry fields, always-present once a layout is migrated. */
export interface GridGeometry {
  x: number;
  y: number;
  w: number;
  h: number;
}

/** A layout entry that carries a resolved position + size. */
export type PositionedEntry = LayoutEntry & GridGeometry;

/**
 * Per-widget starting height (in rows) by display size. Content-dense
 * widgets (heatmap, journey, the bar-list dimensions) start taller so
 * their bodies aren't clipped before the owner resizes. Anything not
 * listed falls back to DEFAULT_ROWS. Values are only the INITIAL height
 * for a never-positioned widget — the owner can resize freely after.
 */
const DEFAULT_ROWS: Readonly<Record<WidgetSize, number>> = {
  compact: 5,
  expanded: 8,
};

// Initial heights are chosen to FIT each widget's bounded summary content
// exactly — a tile never scrolls (list widgets cap to a top-N + "See more")
// and never leaves a big empty gap (stat tiles are short). See the matching
// [minH,maxH] envelope in WIDGET_META.
// EXHAUSTIVE on purpose, matching `WIDGET_META`, whose own comment calls the
// same choice "load-bearing: adding a new widget won't compile until you add
// its meta". As a `Partial<>` this table gave no such guarantee — `corridors`
// shipped with no entry and silently took DEFAULT_ROWS (5/8) rather than the
// list-tile heights its bounds call for. Nothing failed; the tile was just
// the wrong size, which is the hardest kind of bug to notice.
export const ROWS_BY_ID: Record<WidgetId, Record<WidgetSize, number>> = {
  // Content-dense: charts + timelines.
  heatmap: { compact: 8, expanded: 11 },
  journey: { compact: 6, expanded: 10 },
  recent_activity: { compact: 8, expanded: 11 },
  // Sparkline-metric tiles: sparkline + a small readout stack + note.
  travel: { compact: 5, expanded: 11 },
  combat_mission: { compact: 6, expanded: 9 },
  sessions: { compact: 6, expanded: 9 },
  // List / ranking tiles: sized to fit the capped top-N (+ "See more").
  fleet: { compact: 7, expanded: 9 },
  routes: { compact: 7, expanded: 9 },
  // Same bounds as `routes` (minH 5, maxH 9) because expanded is the same
  // shape: a capped ranked list with weight bars. Compact is SHORTER than
  // routes' though — it renders one corridor plus a note, not a list — so it
  // sits at its minH floor rather than copying routes' 7.
  corridors: { compact: 5, expanded: 9 },
  facts: { compact: 5, expanded: 10 },
  locations: { compact: 7, expanded: 9 },
  hangar: { compact: 7, expanded: 9 },
  docking: { compact: 6, expanded: 8 },
  orgs: { compact: 6, expanded: 8 },
  entities: { compact: 4, expanded: 5 },
  loadout: { compact: 5, expanded: 7 },
  // Big-number stat tiles: bounded readouts → short, no wasted space.
  economy: { compact: 4, expanded: 6 },
  spend: { compact: 4, expanded: 6 },
  records: { compact: 4, expanded: 6 },
  stability: { compact: 3, expanded: 5 },
  lives: { compact: 4, expanded: 6 },
  objectives: { compact: 4, expanded: 6 },
  contracts: { compact: 4, expanded: 6 },
};

/** Clamp helper. */
export function clamp(value: number, lo: number, hi: number): number {
  if (Number.isNaN(value)) return lo;
  return Math.min(hi, Math.max(lo, value));
}

/** Legacy 4-col span → 24-col width. */
export function widthFromSpan(span4: number): number {
  return clamp(Math.round(span4) * 6, MIN_W, GRID_COLS);
}

/** Initial row height for a widget at a given size. */
export function rowsFor(id: string, size: WidgetSize): number {
  const perId = ROWS_BY_ID[id as WidgetId];
  return (perId ?? DEFAULT_ROWS)[size];
}

/**
 * Grid width at which a tile is wide enough to hold its expanded body.
 * Half the 24-column grid: a tile occupying half the dashboard is asking
 * to show more than a one-line summary.
 */
export const EXPAND_AT_W = 12;

/**
 * The size a tile should actually RENDER at, given its stored flag and its
 * real width.
 *
 * `cycleSize` only flips a stored `compact | expanded` flag, and its control
 * appears solely in edit mode — so dragging a tile wider used to produce a
 * bigger box holding the same compact body. The tile's real dimensions never
 * reached `body(data, ctx, size)`. Reported as "corridors not expanding past
 * top corridor".
 *
 * WIDTH is the input, deliberately. `fitRows` already derives HEIGHT from
 * measured content, so deriving size from height would feed back on itself —
 * bigger body, taller tile, bigger body. Width is set by the owner and never
 * auto-fitted, so it is a free variable.
 *
 * MONOTONIC: it can only ever ADD content. A tile the owner explicitly
 * expanded stays expanded however narrow it is, so this never strips content
 * out of a layout someone already arranged.
 */
export function effectiveSize(stored: WidgetSize, w: number): WidgetSize {
  return stored === 'expanded' || w >= EXPAND_AT_W ? 'expanded' : 'compact';
}

/** Type guard: does this entry already carry a full geometry? */
export function hasGeometry(entry: LayoutEntry): entry is PositionedEntry {
  return (
    typeof entry.x === 'number' &&
    typeof entry.y === 'number' &&
    typeof entry.w === 'number' &&
    typeof entry.h === 'number'
  );
}

/** Snap step in grid cells. Snap ON → move/resize in 2-cell steps
 *  (aligns to the old 12-col rhythm); snap OFF → 1-cell steps, doubling
 *  the reachable anchor points. */
export function snapStep(snapOn: boolean): number {
  return snapOn ? 2 : 1;
}

/** Round a raw grid value to the active snap step and clamp to bounds. */
export function snapValue(value: number, snapOn: boolean, lo: number, hi: number): number {
  const step = snapStep(snapOn);
  const snapped = Math.round(value / step) * step;
  return clamp(snapped, lo, hi);
}

/** Clamp a width to `[MIN_W, GRID_COLS]`. */
export function clampWidth(w: number): number {
  return clamp(Math.round(w), MIN_W, GRID_COLS);
}

/** Clamp a height to `[MIN_H, +∞)`. */
export function clampHeight(h: number): number {
  return Math.max(MIN_H, Math.round(h));
}

/** Row stride in px: row height + gap (mirrors `--hud-row` 22 + `--hud-gap`
 *  6 in hud.css). A tile spanning `h` rows is `ROW_STRIDE*h - GAP` px tall. */
const ROW_STRIDE = ROW_PX + 6;
/** Chrome above a tile's measured content (header + top/bottom padding). */
const TILE_CHROME_PX = 40;

/**
 * How many rows a tile needs to FIT `contentPx` of measured content without
 * scrolling and without leaving empty space — the heart of content auto-fit.
 * Clamped to the widget's `[minH, maxH]` envelope (so a huge list still caps
 * to its ceiling, where the widget's own "See more" keeps it bounded, and a
 * tiny readout still gets its min-viable floor). Pure → unit-tested.
 */
export function fitRows(contentPx: number, bounds: GridBounds): number {
  const b = normalizeBounds(bounds);
  const rows = Math.ceil((Math.max(0, contentPx) + TILE_CHROME_PX) / ROW_STRIDE);
  // Floor at the GLOBAL minimum, not the widget's min-viable `minH`: content
  // auto-fit measures what the body actually needs, so a widget with little
  // data should shrink all the way down rather than be padded up to a floor
  // meant for the old static-height model. The per-widget `maxH` ceiling is
  // still honoured (a huge list caps there, where its "See more" bounds it).
  return clamp(rows, MIN_H, b.maxH);
}

/**
 * Pack tiles UP to eliminate vertical gaps: keep each tile's column (x) and
 * width (w), assign the smallest y that clears every already-placed tile in
 * its columns (a skyline / masonry pack). This is what turns content-fitted
 * (variable-height) tiles into a gap-free dashboard — without it, shrinking a
 * tile just moves the wasted space below it. Input order sets placement
 * priority; callers sort by (y, x) first for a stable top-to-bottom result.
 * Pure + deterministic.
 */
export function compactUp(
  entries: readonly PositionedEntry[],
): PositionedEntry[] {
  const colBottom = new Array<number>(GRID_COLS).fill(0);
  const ordered = [...entries].sort((a, b) => a.y - b.y || a.x - b.x);
  return ordered.map((e) => {
    const x0 = clamp(e.x, 0, GRID_COLS - 1);
    const x1 = Math.min(GRID_COLS, x0 + e.w);
    let y = 0;
    for (let c = x0; c < x1; c++) y = Math.max(y, colBottom[c]);
    for (let c = x0; c < x1; c++) colBottom[c] = y + e.h;
    return { ...e, y };
  });
}

/** Normalise a per-widget envelope so a malformed bound (min > max, or a
 *  value outside the grid) can never invert the clamp. Widened to the
 *  absolute [MIN_W, GRID_COLS] / [MIN_H, MAX_H] rails. */
export function normalizeBounds(b: GridBounds): GridBounds {
  const minW = clamp(b.minW, MIN_W, GRID_COLS);
  const maxW = clamp(Math.max(b.maxW, minW), minW, GRID_COLS);
  const minH = clamp(b.minH, MIN_H, MAX_H);
  const maxH = clamp(Math.max(b.maxH, minH), minH, MAX_H);
  return { minW, minH, maxW, maxH };
}

/** Clamp a width to the widget's `[minW, maxW]` envelope. */
export function clampWidthTo(w: number, bounds: GridBounds): number {
  const b = normalizeBounds(bounds);
  return clamp(Math.round(w), b.minW, b.maxW);
}

/** Clamp a height to the widget's `[minH, maxH]` envelope. */
export function clampHeightTo(h: number, bounds: GridBounds): number {
  const b = normalizeBounds(bounds);
  return clamp(Math.round(h), b.minH, b.maxH);
}

/** Clamp an x origin so the tile stays inside the grid given its width. */
export function clampX(x: number, w: number): number {
  return clamp(Math.round(x), 0, GRID_COLS - clampWidth(w));
}

/** Clamp a y origin to the non-negative rows. */
export function clampY(y: number): number {
  return Math.max(0, Math.round(y));
}

/**
 * Fill geometry for every entry that lacks it, leaving already-positioned
 * entries untouched.
 *
 *  - Entries with a full `{x,y,w,h}` keep it verbatim (idempotent — a
 *    round-trip through the store never disturbs a customised layout).
 *  - Entries missing geometry are packed left-to-right / top-to-bottom on
 *    the 24-col grid, ENABLED first then disabled, starting below any
 *    already-positioned rows. Packing enabled-first means a default
 *    layout (whose disabled widgets trail the enabled ones) never leaves
 *    holes among the visible tiles.
 *
 * `spanOf` maps a widget id to its legacy 4-col span (the width heuristic).
 * The output preserves the INPUT ARRAY ORDER — only geometry is added — so
 * the DOM render order and `render_diff` "moved" audit semantics are stable.
 */
export function ensureGeometry(
  entries: readonly LayoutEntry[],
  spanOf: (id: string) => number,
  boundsOf?: (id: string) => GridBounds,
): PositionedEntry[] {
  // When per-widget bounds are supplied, width/height clamp to each
  // widget's envelope; otherwise fall back to the grid-wide rails so the
  // legacy two-arg call stays byte-identical.
  const clampW = (w: number, id: string) =>
    boundsOf ? clampWidthTo(w, boundsOf(id)) : clampWidth(w);
  const clampH = (h: number, id: string) =>
    boundsOf ? clampHeightTo(h, boundsOf(id)) : clampHeight(h);

  const positioned = new Map<string, GridGeometry>();
  let baseBottom = 0;

  // Pass 1: keep existing geometry, remember the lowest free row.
  for (const entry of entries) {
    if (hasGeometry(entry)) {
      const w = clampW(entry.w as number, entry.id);
      const geo: GridGeometry = {
        x: clampX(entry.x as number, w),
        y: clampY(entry.y as number),
        w,
        h: clampH(entry.h as number, entry.id),
      };
      positioned.set(entry.id, geo);
      baseBottom = Math.max(baseBottom, geo.y + geo.h);
    }
  }

  // Pass 2: pack the un-positioned entries, enabled first.
  const missing = entries.filter((e) => !hasGeometry(e));
  const order = [
    ...missing.filter((e) => e.enabled),
    ...missing.filter((e) => !e.enabled),
  ];

  let cursorX = 0;
  let cursorY = baseBottom;
  let rowTallest = 0;
  for (const entry of order) {
    const w = clampW(widthFromSpan(spanOf(entry.id)), entry.id);
    const h = clampH(rowsFor(entry.id, entry.size), entry.id);
    if (cursorX + w > GRID_COLS) {
      cursorY += rowTallest;
      cursorX = 0;
      rowTallest = 0;
    }
    positioned.set(entry.id, { x: cursorX, y: cursorY, w, h });
    cursorX += w;
    rowTallest = Math.max(rowTallest, h);
  }

  return entries.map((entry) => {
    const geo = positioned.get(entry.id)!;
    return { ...entry, ...geo };
  });
}

/** Do two rectangles overlap on the grid? */
function overlaps(a: GridGeometry, b: GridGeometry): boolean {
  return (
    a.x < b.x + b.w &&
    a.x + a.w > b.x &&
    a.y < b.y + b.h &&
    a.y + a.h > b.y
  );
}

/**
 * Resolve overlaps introduced by moving/resizing `movedId`: every OTHER
 * tile that now intersects the moved tile is pushed straight down until
 * it clears, cascading. The moved tile keeps the position the user chose
 * (free placement); the rest reflow so nothing renders stacked on top of
 * anything else. Pure + deterministic.
 */
export function resolveCollisions(
  entries: readonly PositionedEntry[],
  movedId: string,
): PositionedEntry[] {
  const moved = entries.find((e) => e.id === movedId);
  if (!moved) return [...entries];

  // Sort the others by y so a downward push cascades cleanly.
  const others = entries
    .filter((e) => e.id !== movedId)
    .sort((a, b) => a.y - b.y || a.x - b.x);

  const placed: PositionedEntry[] = [{ ...moved }];
  for (const entry of others) {
    let next: PositionedEntry = { ...entry };
    let guard = 0;
    while (placed.some((p) => overlaps(next, p)) && guard < 1000) {
      // Push below the lowest tile it currently collides with.
      const bottom = Math.max(
        ...placed.filter((p) => overlaps(next, p)).map((p) => p.y + p.h),
      );
      next = { ...next, y: bottom };
      guard += 1;
    }
    placed.push(next);
  }

  // Restore original array order (geometry updated).
  const byId = new Map(placed.map((p) => [p.id, p]));
  return entries.map((e) => byId.get(e.id) ?? e);
}

/** Lowest occupied row across the layout (grid height in rows). */
export function gridBottom(entries: readonly PositionedEntry[]): number {
  return entries.reduce((max, e) => Math.max(max, e.y + e.h), 0);
}

/** Immutable single-entry geometry patch. */
export function patchGeometry(
  entries: readonly LayoutEntry[],
  id: string,
  patch: Partial<GridGeometry>,
): LayoutEntry[] {
  return entries.map((e) => (e.id === id ? { ...e, ...patch } : e));
}

/** Centered column width used by the focus projection. Wide enough to
 *  enlarge a focused widget well past its scattered dashboard size, while
 *  leaving symmetric side gutters on the 24-col grid. */
export const FOCUS_W = 16;
/** Left origin that centers a `FOCUS_W` column on the grid. */
export const FOCUS_X = Math.floor((GRID_COLS - FOCUS_W) / 2);
/** Minimum focused-tile height so a single focused widget reads large. */
export const FOCUS_MIN_H = 8;

/**
 * Project a set of entries into a single, centered, enlarged column —
 * the "focus" (lens) view. The caller passes the ALREADY-FILTERED entries
 * it wants in focus (matching the active lens, enabled, renderable); this
 * stacks them top-to-bottom in input order at a fixed width, each at least
 * `FOCUS_MIN_H` tall.
 *
 * This is a pure VIEW transformation over positioned entries — it returns
 * fresh objects and never mutates or persists the saved layout, so
 * clearing the lens restores the exact stored geometry. Widen once (`w`),
 * center once (`x`), stack (`y`); height grows to the larger of the
 * widget's own height and the focus minimum so tall widgets keep their
 * room.
 */
export function focusLayout(
  entries: readonly PositionedEntry[],
): PositionedEntry[] {
  let cursorY = 0;
  return entries.map((entry) => {
    const h = Math.max(entry.h, FOCUS_MIN_H);
    const placed: PositionedEntry = {
      ...entry,
      x: FOCUS_X,
      y: cursorY,
      w: FOCUS_W,
      h,
    };
    cursorY += h;
    return placed;
  });
}
