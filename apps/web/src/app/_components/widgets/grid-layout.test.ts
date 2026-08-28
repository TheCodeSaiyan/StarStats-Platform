import { describe, it, expect } from 'vitest';
import type { LayoutEntry } from '@/lib/api';
import { WIDGET_META } from './widget-meta';
import type { WidgetId } from './types';
import {
  ROWS_BY_ID,
  rowsFor,
  effectiveSize,
  EXPAND_AT_W,
  GRID_COLS,
  MIN_W,
  MIN_H,
  MAX_H,
  ensureGeometry,
  hasGeometry,
  snapStep,
  snapValue,
  widthFromSpan,
  clampX,
  clampWidth,
  clampWidthTo,
  clampHeightTo,
  fitRows,
  compactUp,
  normalizeBounds,
  focusLayout,
  FOCUS_W,
  FOCUS_X,
  FOCUS_MIN_H,
  resolveCollisions,
  gridBottom,
  patchGeometry,
  type GridBounds,
  type PositionedEntry,
} from './grid-layout';
import { boundsForWidget } from './widget-meta';

// A representative legacy span map (mirrors TILE_SPANS shape).
const SPANS: Record<string, number> = {
  heatmap: 4,
  travel: 2,
  sessions: 2,
  orgs: 1,
  economy: 1,
  entities: 1,
};
const spanOf = (id: string) => SPANS[id] ?? 1;

describe('widthFromSpan', () => {
  it('maps the 4-col span onto the 24-col grid (×6)', () => {
    expect(widthFromSpan(4)).toBe(24);
    expect(widthFromSpan(2)).toBe(12);
    expect(widthFromSpan(1)).toBe(6);
  });
  it('never returns below the minimum width', () => {
    expect(widthFromSpan(0)).toBe(MIN_W);
  });
  it('never overflows the grid', () => {
    expect(widthFromSpan(99)).toBe(GRID_COLS);
  });
});

describe('snapping', () => {
  it('steps in 2 cells when snapping is on, 1 when off', () => {
    expect(snapStep(true)).toBe(2);
    expect(snapStep(false)).toBe(1);
  });
  it('rounds to the active step and clamps', () => {
    // snap ON: 5 → nearest even = 6 (within 0..24)
    expect(snapValue(5, true, 0, GRID_COLS)).toBe(6);
    // snap OFF: 5 → 5
    expect(snapValue(5, false, 0, GRID_COLS)).toBe(5);
    // clamp to hi
    expect(snapValue(99, false, 0, GRID_COLS)).toBe(GRID_COLS);
    // clamp to lo
    expect(snapValue(-4, false, 0, GRID_COLS)).toBe(0);
  });
});

describe('ensureGeometry — BACKWARD COMPATIBILITY', () => {
  const legacy: LayoutEntry[] = [
    { id: 'heatmap', enabled: true, size: 'expanded' },
    { id: 'travel', enabled: true, size: 'compact' },
    { id: 'orgs', enabled: true, size: 'compact' },
    { id: 'economy', enabled: false, size: 'compact' },
    { id: 'entities', enabled: true, size: 'compact' },
  ];

  it('gives every legacy entry a full geometry', () => {
    const out = ensureGeometry(legacy, spanOf);
    expect(out).toHaveLength(legacy.length);
    for (const e of out) {
      expect(hasGeometry(e)).toBe(true);
    }
  });

  it('preserves the original array order (only geometry is added)', () => {
    const out = ensureGeometry(legacy, spanOf);
    expect(out.map((e) => e.id)).toEqual(legacy.map((e) => e.id));
    // id/enabled/size are untouched.
    out.forEach((e, i) => {
      expect(e.id).toBe(legacy[i].id);
      expect(e.enabled).toBe(legacy[i].enabled);
      expect(e.size).toBe(legacy[i].size);
    });
  });

  it('packs enabled widgets before disabled ones so the default has no holes', () => {
    const out = ensureGeometry(legacy, spanOf);
    const enabledYs = out.filter((e) => e.enabled).map((e) => e.y);
    const disabled = out.find((e) => e.id === 'economy')!;
    // The disabled tile is placed at or below every enabled tile.
    expect(disabled.y).toBeGreaterThanOrEqual(Math.max(...enabledYs));
  });

  it('never positions a tile outside the grid horizontally', () => {
    const out = ensureGeometry(legacy, spanOf);
    for (const e of out) {
      expect(e.x).toBeGreaterThanOrEqual(0);
      expect(e.x + e.w).toBeLessThanOrEqual(GRID_COLS);
    }
  });

  it('is idempotent: re-running keeps positioned entries byte-identical (ROUND-TRIP)', () => {
    const once = ensureGeometry(legacy, spanOf);
    // Simulate a store round-trip: the positioned entries come back as
    // plain LayoutEntry[] carrying x/y/w/h.
    const roundTripped: LayoutEntry[] = once.map((e) => ({
      id: e.id,
      enabled: e.enabled,
      size: e.size,
      x: e.x,
      y: e.y,
      w: e.w,
      h: e.h,
    }));
    const twice = ensureGeometry(roundTripped, spanOf);
    expect(twice).toEqual(once);
  });

  it('places only the newly-appended widget, leaving customised ones fixed', () => {
    // A user who customised heatmap, then a new widget "sessions" was
    // appended by the registry projection with NO geometry.
    const mixed: LayoutEntry[] = [
      { id: 'heatmap', enabled: true, size: 'expanded', x: 6, y: 2, w: 12, h: 8 },
      { id: 'sessions', enabled: true, size: 'compact' },
    ];
    const out = ensureGeometry(mixed, spanOf);
    const heat = out.find((e) => e.id === 'heatmap')!;
    expect({ x: heat.x, y: heat.y, w: heat.w, h: heat.h }).toEqual({
      x: 6,
      y: 2,
      w: 12,
      h: 8,
    });
    // The new widget lands below the customised heatmap (y >= 2 + 8).
    const sessions = out.find((e) => e.id === 'sessions')!;
    expect(sessions.y).toBeGreaterThanOrEqual(10);
    expect(hasGeometry(sessions)).toBe(true);
  });
});

describe('clamp helpers', () => {
  it('clampWidth keeps width within [MIN_W, GRID_COLS]', () => {
    expect(clampWidth(1)).toBe(MIN_W);
    expect(clampWidth(99)).toBe(GRID_COLS);
    expect(clampWidth(12)).toBe(12);
  });
  it('clampX keeps the tile fully inside the grid', () => {
    expect(clampX(20, 12)).toBe(GRID_COLS - 12); // 12
    expect(clampX(-3, 6)).toBe(0);
  });
});

describe('resolveCollisions', () => {
  it('pushes overlapping tiles below the moved tile', () => {
    const entries: PositionedEntry[] = [
      { id: 'a', enabled: true, size: 'compact', x: 0, y: 0, w: 12, h: 6 },
      { id: 'b', enabled: true, size: 'compact', x: 0, y: 0, w: 12, h: 6 },
    ];
    // 'a' was just moved on top of 'b'.
    const out = resolveCollisions(entries, 'a');
    const a = out.find((e) => e.id === 'a')!;
    const b = out.find((e) => e.id === 'b')!;
    expect(a.y).toBe(0); // moved tile keeps its spot
    expect(b.y).toBeGreaterThanOrEqual(a.y + a.h); // pushed clear
  });

  it('leaves non-overlapping tiles where they are', () => {
    const entries: PositionedEntry[] = [
      { id: 'a', enabled: true, size: 'compact', x: 0, y: 0, w: 6, h: 6 },
      { id: 'b', enabled: true, size: 'compact', x: 12, y: 0, w: 6, h: 6 },
    ];
    const out = resolveCollisions(entries, 'a');
    expect(out.find((e) => e.id === 'b')!.y).toBe(0);
  });

  it('preserves array order', () => {
    const entries: PositionedEntry[] = [
      { id: 'a', enabled: true, size: 'compact', x: 0, y: 0, w: 12, h: 6 },
      { id: 'b', enabled: true, size: 'compact', x: 0, y: 0, w: 12, h: 6 },
    ];
    expect(resolveCollisions(entries, 'b').map((e) => e.id)).toEqual(['a', 'b']);
  });
});

describe('gridBottom', () => {
  it('returns the lowest occupied row', () => {
    const entries: PositionedEntry[] = [
      { id: 'a', enabled: true, size: 'compact', x: 0, y: 0, w: 6, h: 6 },
      { id: 'b', enabled: true, size: 'compact', x: 6, y: 4, w: 6, h: 8 },
    ];
    expect(gridBottom(entries)).toBe(12); // 4 + 8
  });
});

describe('per-widget size bounds', () => {
  const bounds: GridBounds = { minW: 6, minH: 4, maxW: 12, maxH: 10 };

  it('clampWidthTo keeps width within the widget envelope', () => {
    expect(clampWidthTo(2, bounds)).toBe(6); // below min → min
    expect(clampWidthTo(99, bounds)).toBe(12); // above max → max
    expect(clampWidthTo(8, bounds)).toBe(8); // in range
  });

  it('clampHeightTo keeps height within the widget envelope', () => {
    expect(clampHeightTo(1, bounds)).toBe(4); // below min → min
    expect(clampHeightTo(99, bounds)).toBe(10); // above max → max
    expect(clampHeightTo(7, bounds)).toBe(7);
  });

  it('normalizeBounds repairs an inverted / out-of-grid envelope', () => {
    const n = normalizeBounds({ minW: 20, minH: 30, maxW: 4, maxH: 2 });
    expect(n.minW).toBeGreaterThanOrEqual(MIN_W);
    expect(n.maxW).toBeGreaterThanOrEqual(n.minW);
    expect(n.maxW).toBeLessThanOrEqual(GRID_COLS);
    expect(n.minH).toBeGreaterThanOrEqual(MIN_H);
    expect(n.maxH).toBeGreaterThanOrEqual(n.minH);
    expect(n.maxH).toBeLessThanOrEqual(MAX_H);
  });

  it('ensureGeometry with boundsOf clamps a too-tall stored tile to its max', () => {
    const boundsOf = () => bounds;
    const entries = [
      { id: 'x', enabled: true, size: 'compact' as const, x: 0, y: 0, w: 24, h: 99 },
    ];
    const out = ensureGeometry(entries, () => 2, boundsOf);
    expect(out[0].w).toBe(12); // clamped to maxW
    expect(out[0].h).toBe(10); // clamped to maxH
  });

  it('ensureGeometry without boundsOf is unchanged (legacy rails)', () => {
    const entries = [
      { id: 'x', enabled: true, size: 'compact' as const, x: 0, y: 0, w: 24, h: 30 },
    ];
    const out = ensureGeometry(entries, () => 2);
    expect(out[0].w).toBe(24);
    expect(out[0].h).toBe(30);
  });

  // Regression: a previously-saved tile is re-clamped into the widget's
  // [minH, maxH] envelope on every load (Pass 1 via boundsOf). This heals
  // BOTH failure modes on already-customised dashboards — a too-short
  // stored tile lifts to its floor, and a too-tall stored tile (the
  // "wasted empty space" case) drops to its ceiling.
  it('re-clamps a legacy saved tile into the widget envelope on load', () => {
    const boundsOf = (id: string) => boundsForWidget(id);
    const b = boundsForWidget('fleet');
    const tall = ensureGeometry(
      [{ id: 'fleet', enabled: true, size: 'compact' as const, x: 0, y: 0, w: 8, h: 99 }],
      () => 2,
      boundsOf,
    );
    expect(tall[0].h).toBe(b.maxH);
    const short = ensureGeometry(
      [{ id: 'fleet', enabled: true, size: 'compact' as const, x: 0, y: 0, w: 8, h: 1 }],
      () => 2,
      boundsOf,
    );
    expect(short[0].h).toBe(b.minH);
  });
});

describe('fitRows (content auto-fit)', () => {
  const bounds: GridBounds = { minW: 6, minH: 3, maxW: 12, maxH: 10 };

  // The numbers below moved when the row model was corrected against what the
  // browser actually renders. The old expectations encoded `28px stride, 40px
  // chrome` and a tile height of `stride*rows - gap`; measured on /u/[handle],
  // the grid is 24px rows + 6px gaps (stride 30), the chrome is 59px, and a
  // tile spanning N rows renders `stride*(N-1)` — confirmed at three
  // independent spans (4 -> 90px, 6 -> 150px, 7 -> 180px). Every tile was
  // therefore sized a row short and scrolled. These assertions are corrected,
  // not relaxed: each still pins an exact row count.

  it('floors at the global minimum for empty content', () => {
    // 30px stride, 60px chrome: ceil(60/30) + 1 = 3.
    expect(fitRows(0, bounds)).toBe(3);
    // A single readout needs one row more than the floor: ceil(75/30) + 1 = 4.
    expect(fitRows(15, bounds)).toBe(4);
  });

  it('grows to fit taller content', () => {
    // 200px content: ceil((200 + 60) / 30) + 1 = 10 rows, which is also this
    // fixture's maxH — so it lands exactly on the ceiling.
    expect(fitRows(200, bounds)).toBe(10);
  });

  it('caps oversized content at the widget ceiling (its See-more keeps it bounded)', () => {
    expect(fitRows(5000, bounds)).toBe(10); // clamped to maxH
  });

  it('is monotonic — more content never means fewer rows', () => {
    let prev = 0;
    for (let px = 0; px <= 400; px += 20) {
      const r = fitRows(px, bounds);
      expect(r).toBeGreaterThanOrEqual(prev);
      prev = r;
    }
  });
});

describe('compactUp (masonry pack)', () => {
  const e = (id: string, x: number, y: number, w: number, h: number): PositionedEntry => ({
    id,
    enabled: true,
    size: 'compact',
    x,
    y,
    w,
    h,
  });

  it('pulls a tile up to sit directly below the one above it in the same column', () => {
    const out = compactUp([e('a', 0, 0, 6, 4), e('b', 0, 20, 6, 3)]);
    const b = out.find((t) => t.id === 'b')!;
    expect(b.y).toBe(4); // packed right under a (height 4), not left at y=20
  });

  it('packs independent columns independently', () => {
    const out = compactUp([e('a', 0, 0, 6, 5), e('b', 6, 0, 6, 2), e('c', 6, 30, 6, 3)]);
    expect(out.find((t) => t.id === 'a')!.y).toBe(0);
    expect(out.find((t) => t.id === 'b')!.y).toBe(0);
    expect(out.find((t) => t.id === 'c')!.y).toBe(2); // under b (height 2), not a
  });

  it('never overlaps overlapping-column tiles', () => {
    const out = compactUp([e('a', 0, 0, 12, 4), e('b', 6, 10, 12, 3)]);
    // b spans cols 6-17, overlapping a (0-11) → must sit below a
    expect(out.find((t) => t.id === 'b')!.y).toBe(4);
  });
});

describe('focusLayout (lens projection)', () => {
  const grid: PositionedEntry[] = [
    { id: 'a', enabled: true, size: 'compact', x: 0, y: 5, w: 6, h: 6 },
    { id: 'b', enabled: true, size: 'compact', x: 12, y: 0, w: 6, h: 12 },
  ];

  it('stacks entries in a single centered, widened column', () => {
    const out = focusLayout(grid);
    expect(out.map((e) => e.id)).toEqual(['a', 'b']);
    for (const e of out) {
      expect(e.x).toBe(FOCUS_X);
      expect(e.w).toBe(FOCUS_W);
      expect(e.x + e.w).toBeLessThanOrEqual(GRID_COLS);
    }
    // First at the top; second stacked directly below the first.
    expect(out[0].y).toBe(0);
    expect(out[1].y).toBe(out[0].h);
  });

  it('enlarges short tiles to at least the focus minimum height', () => {
    const out = focusLayout(grid);
    expect(out[0].h).toBe(FOCUS_MIN_H); // 6 → 8
    expect(out[1].h).toBe(12); // already taller than the minimum, kept
  });

  it('does not mutate the input entries (view-only projection)', () => {
    const before = JSON.parse(JSON.stringify(grid));
    focusLayout(grid);
    expect(grid).toEqual(before);
  });

  it('returns an empty column for no matches', () => {
    expect(focusLayout([])).toEqual([]);
  });
});

describe('patchGeometry', () => {
  it('updates only the targeted entry immutably', () => {
    const entries: LayoutEntry[] = [
      { id: 'a', enabled: true, size: 'compact', x: 0, y: 0, w: 6, h: 6 },
      { id: 'b', enabled: true, size: 'compact', x: 6, y: 0, w: 6, h: 6 },
    ];
    const out = patchGeometry(entries, 'a', { x: 12 });
    expect(out.find((e) => e.id === 'a')!.x).toBe(12);
    expect(out.find((e) => e.id === 'b')!.x).toBe(6);
    expect(out).not.toBe(entries); // new array
    expect(entries[0].x).toBe(0); // original untouched
  });
});

// `WIDGET_META` is `Record<WidgetId, WidgetMeta>` and its own comment calls
// that "load-bearing: adding a new widget won't compile until you add its
// meta". `ROWS_BY_ID` was `Partial<Record<...>>`, so it carried no such
// guarantee — and `corridors` shipped without an entry, silently taking
// DEFAULT_ROWS (5/8) instead of the list-tile heights its identical bounds
// to `routes` call for. Nothing failed; the tile was just the wrong size.
//
// The type is exhaustive now, so this can't recur silently. This test keeps
// the invariant visible at runtime too, since `rowsFor` takes a string.
describe('ROWS_BY_ID covers the registry', () => {
  it('has an explicit initial height for every registered widget', () => {
    const ids = Object.keys(WIDGET_META) as WidgetId[];
    const missing = ids.filter((id) => ROWS_BY_ID[id] === undefined);
    expect(missing).toEqual([]);
  });

  it('keeps every initial height inside that widget\'s own bounds', () => {
    // An initial height outside [minH, maxH] is clipped or padded on first
    // render, before the owner has touched anything.
    const offenders: string[] = [];
    for (const [id, meta] of Object.entries(WIDGET_META)) {
      for (const size of ['compact', 'expanded'] as const) {
        const rows = rowsFor(id, size);
        if (rows < meta.bounds.minH || rows > meta.bounds.maxH) {
          offenders.push(`${id}.${size}=${rows} outside [${meta.bounds.minH},${meta.bounds.maxH}]`);
        }
      }
    }
    expect(offenders).toEqual([]);
  });
});

// Reported as "corridors not expanding past top corridor". `cycleSize`
// flips a stored discrete flag and its control only renders in edit mode,
// so dragging a tile wider produced a bigger box holding the same compact
// body — the tile's real dimensions never reached `body(data, ctx, size)`.
//
// Width is the safe input. `fitRows` already derives HEIGHT from content,
// so deriving size from height would feed back on itself; width is set by
// the owner and never auto-fitted.
describe('effectiveSize (content responds to tile width)', () => {
  it('expands a wide tile even when the stored flag says compact', () => {
    expect(effectiveSize('compact', EXPAND_AT_W)).toBe('expanded');
    expect(effectiveSize('compact', GRID_COLS)).toBe('expanded');
  });

  it('leaves a narrow tile compact', () => {
    expect(effectiveSize('compact', EXPAND_AT_W - 1)).toBe('compact');
    expect(effectiveSize('compact', MIN_W)).toBe('compact');
  });

  // Monotonic: it may only ADD content. A stored `expanded` must survive a
  // narrow tile, or this change would silently strip content from layouts
  // people already arranged.
  it('never downgrades a tile the owner explicitly expanded', () => {
    expect(effectiveSize('expanded', MIN_W)).toBe('expanded');
    expect(effectiveSize('expanded', EXPAND_AT_W - 1)).toBe('expanded');
  });
});
