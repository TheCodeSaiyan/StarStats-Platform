'use client';

import React, {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  useTransition,
  type ReactNode,
} from 'react';
import { LENSES, type Lens, widgetMatchesLens } from '@/lib/lens';
import {
  DndContext,
  type DragEndEvent,
  KeyboardSensor,
  PointerSensor,
  useDraggable,
  useSensor,
  useSensors,
} from '@dnd-kit/core';
import { CSS } from '@dnd-kit/utilities';
import type { LayoutEntry, LayoutSurface } from '@/lib/api';
import type { WidgetId, WidgetSize } from './types';
import { WIDGET_META, boundsForWidget } from './widget-meta';
import { useEditMode } from './useEditMode';
import { saveProfileLayoutAction } from '@/app/_actions/profile-layout';
import { Tile } from '@/components/hud/Tile';
import { TILE_SPANS } from '@/components/hud/tile-spans';
import {
  GRID_COLS,
  type GridBounds,
  clampHeightTo,
  clampWidthTo,
  clampX,
  clampY,
  compactUp,
  ensureGeometry,
  fitRows,
  focusLayout,
  gridBottom,
  hasGeometry,
  patchGeometry,
  resolveCollisions,
  rowsFor,
  snapStep,
  snapValue,
  widthFromSpan,
  type PositionedEntry,
} from './grid-layout';

/** Row gap of `.hud-freegrid` in px — mirrors `--hud-gap` in hud.css so
 *  px→row conversion during drag/resize lines up with what the user sees. */
const GRID_GAP_PX = 6;
/** Row stride (row height + gap) for px→cell conversion during drag. Row
 *  height mirrors `--hud-row` (22px) in hud.css. */
const ROW_STRIDE_PX = 22 + GRID_GAP_PX;

export interface RenderedWidget {
  id: string;
  eyebrow: string;
  title: string;
  body: ReactNode;
  isRangeAware?: boolean;
  /** True when the widget returned no data (renders a compact "no signal"
   *  placeholder). Content auto-fit already collapses these to a short tile
   *  (the placeholder is short); the flag is kept for potential styling. */
  empty?: boolean;
}

interface Props {
  initialLayout: LayoutEntry[];
  rendered: ReadonlyMap<string, RenderedWidget>;
  surface?: LayoutSurface;
  lensEnabled?: boolean;
}

const spanOf = (id: string) => TILE_SPANS[id as WidgetId] ?? 1;
const boundsOf = (id: string): GridBounds => boundsForWidget(id);

/**
 * Owner-side editable widget grid (M7 free drag/resize, v2).
 *
 * Reads edit mode from the URL via `useEditMode`. When editing, the grid
 * becomes a free drag/resize canvas: each tile can be dragged to any of the
 * 24 columns, resized within its per-widget min/max envelope, or removed to
 * the palette; a snap toggle switches between coarse (2-cell) and fine
 * (1-cell) anchor points; and an "Add widget" palette re-adds any widget
 * that isn't currently on the grid.
 *
 * The Focus lens (home surface only, view mode) reprojects the matching
 * widgets into a single centered, enlarged column via `focusLayout` — a
 * pure VIEW transform that never touches the saved layout, so clearing the
 * lens restores the exact stored geometry.
 *
 * Backward compatibility: `ensureGeometry` derives a free-grid position for
 * any legacy `{id,enabled,size}` layout, so a user who never opens the
 * editor sees an unchanged dashboard.
 */
export function SortableProfileWidgets({
  initialLayout,
  rendered,
  surface = 'profile',
  lensEnabled = false,
}: Props) {
  // State is stored as LayoutEntry[] (what we persist); geometry is
  // always resolved for rendering via `grid` below.
  const [layout, setLayout] = useState<LayoutEntry[]>(() =>
    ensureGeometry(initialLayout, spanOf, boundsOf),
  );
  const [activeLens, setActiveLens] = useState<Lens>('all');
  const [snapOn, setSnapOn] = useState(true);
  const { isEditing, setEditing } = useEditMode();
  const [, startTransition] = useTransition();
  const gridRef = useRef<HTMLDivElement | null>(null);

  // Measured natural content height (px) per widget id, reported by each
  // tile via ResizeObserver. Drives content auto-fit: in view mode a tile's
  // row-span is derived from what its content actually needs, so no tile
  // scrolls (content exceeds) or wastes space (content falls short).
  //
  // CLICK-STABILITY: ResizeObserver callbacks fire OUTSIDE React's batching,
  // so 19 tiles reporting on mount (+ async image loads on real data) would
  // each trigger a separate re-layout — the grid keeps shifting under the
  // cursor and drill-down clicks miss (they land mid-relayout). So reports
  // are coalesced into ONE state update per animation frame: the grid
  // re-lays-out at most once per frame and settles immediately, keeping
  // links clickable.
  const [measured, setMeasured] = useState<Record<string, number>>({});
  const pendingRef = useRef<Record<string, number>>({});
  const rafRef = useRef<number | null>(null);
  // Once the layout has SETTLED (initial fonts/images have loaded and the
  // fit stops changing), FREEZE it: stop accepting further measurements so
  // the grid never re-compacts under the cursor mid-interaction. A late
  // reflow (a lazy image) then just scrolls within its tile rather than
  // shifting every tile and eating a drill-down click. Re-settles on a
  // deliberate content change (range switch → new `rendered` map).
  const settledRef = useRef(false);
  useEffect(() => {
    settledRef.current = false;
    const t = setTimeout(() => {
      settledRef.current = true;
    }, 1500);
    return () => clearTimeout(t);
  }, [rendered]);
  const flushMeasures = useCallback(() => {
    rafRef.current = null;
    const pending = pendingRef.current;
    pendingRef.current = {};
    setMeasured((prev) => {
      let changed = false;
      const next = { ...prev };
      for (const id in pending) {
        if (Math.abs((prev[id] ?? -1) - pending[id]) >= 1) {
          next[id] = pending[id];
          changed = true;
        }
      }
      return changed ? next : prev;
    });
  }, []);
  const handleMeasure = useCallback(
    (id: string, px: number) => {
      // Frozen after settle — ignore late reflows so tiles don't shift.
      if (settledRef.current) return;
      pendingRef.current[id] = px;
      if (rafRef.current == null && typeof requestAnimationFrame !== 'undefined') {
        rafRef.current = requestAnimationFrame(flushMeasures);
      } else if (typeof requestAnimationFrame === 'undefined') {
        flushMeasures(); // jsdom / SSR — apply immediately
      }
    },
    [flushMeasures],
  );
  useEffect(
    () => () => {
      if (rafRef.current != null) cancelAnimationFrame(rafRef.current);
    },
    [],
  );

  const sensors = useSensors(
    useSensor(PointerSensor, { activationConstraint: { distance: 4 } }),
    useSensor(KeyboardSensor),
  );

  /** Fully-positioned view of the current layout. */
  const grid: PositionedEntry[] = useMemo(
    () => ensureGeometry(layout, spanOf, boundsOf),
    [layout],
  );

  const save = useCallback(
    (next: LayoutEntry[]) => {
      setLayout(next);
      startTransition(async () => {
        await saveProfileLayoutAction(next, surface);
      });
    },
    [surface],
  );

  /** Live (uncommitted) geometry update — used during a resize drag so
   *  the tile tracks the pointer without a save on every frame. */
  const previewGeometry = useCallback(
    (id: string, patch: Partial<PositionedEntry>) => {
      setLayout((prev) => patchGeometry(prev, id, patch));
    },
    [],
  );

  /** Commit a geometry change: clamp to the widget's envelope, resolve
   *  overlaps, then persist. */
  const commitGeometry = useCallback(
    (id: string, patch: Partial<PositionedEntry>) => {
      const based = ensureGeometry(layout, spanOf, boundsOf);
      const patched = ensureGeometry(
        patchGeometry(based, id, patch),
        spanOf,
        boundsOf,
      );
      save(resolveCollisions(patched, id));
    },
    [layout, save],
  );

  /** Add a widget from the palette: enable it and drop it into the first
   *  free row (below the current content), sized within its envelope. */
  const addWidget = useCallback(
    (id: string) => {
      const based = ensureGeometry(layout, spanOf, boundsOf);
      // A widget removed from the grid keeps its x/y/w/h on the stored
      // entry, so re-adding restores exactly where the owner had it.
      // Only a widget that was never positioned gets a fresh bottom-row
      // placement sized to its envelope. Check the ORIGINAL (pre-
      // ensureGeometry) entry — `based` has geometry on every row.
      const remembered = layout.some((e) => e.id === id && hasGeometry(e));
      let next: typeof based;
      if (remembered) {
        next = based.map((e) => (e.id === id ? { ...e, enabled: true } : e));
      } else {
        const b = boundsOf(id);
        const size = based.find((e) => e.id === id)?.size ?? 'compact';
        const w = clampWidthTo(widthFromSpan(spanOf(id)), b);
        const h = clampHeightTo(rowsFor(id, size), b);
        const y = gridBottom(based.filter((e) => e.enabled));
        next = based.map((e) =>
          e.id === id ? { ...e, enabled: true, x: 0, y, w, h } : e,
        );
      }
      // resolveCollisions guards against another widget having occupied
      // the remembered slot while this one was in the palette.
      save(resolveCollisions(ensureGeometry(next, spanOf, boundsOf), id));
    },
    [layout, save],
  );

  /** Remove a widget from the dashboard — it returns to the Add palette.
   *  Distinct from a destructive delete: the entry stays in the layout so
   *  its geometry is remembered if the owner re-adds it. */
  const removeWidget = useCallback(
    (id: string) =>
      save(layout.map((e) => (e.id === id ? { ...e, enabled: false } : e))),
    [layout, save],
  );

  const cycleSize = (id: string) =>
    save(
      layout.map((e) =>
        e.id === id
          ? {
              ...e,
              size: (e.size === 'compact' ? 'expanded' : 'compact') as WidgetSize,
            }
          : e,
      ),
    );

  /** Convert an on-screen px delta to whole grid cells. */
  const pxToCells = useCallback((dxPx: number, dyPx: number) => {
    const width = gridRef.current?.getBoundingClientRect().width ?? 0;
    const colStride = width > 0 ? width / GRID_COLS : 0;
    const rowStride = ROW_STRIDE_PX;
    return {
      dCols: colStride > 0 ? Math.round(dxPx / colStride) : 0,
      dRows: rowStride > 0 ? Math.round(dyPx / rowStride) : 0,
    };
  }, []);

  const onDragEnd = useCallback(
    (e: DragEndEvent) => {
      const id = String(e.active.id);
      const cur = grid.find((g) => g.id === id);
      if (!cur) return;
      const { dCols, dRows } = pxToCells(e.delta.x, e.delta.y);
      if (dCols === 0 && dRows === 0) return;
      const nx = clampX(snapValue(cur.x + dCols, snapOn, 0, GRID_COLS), cur.w);
      const ny = clampY(
        snapValue(cur.y + dRows, snapOn, 0, Number.MAX_SAFE_INTEGER),
      );
      commitGeometry(id, { x: nx, y: ny });
    },
    [grid, pxToCells, snapOn, commitGeometry],
  );

  const showLens = lensEnabled && !isEditing;
  const focusing = showLens && activeLens !== 'all';

  // Tiles currently on the dashboard (enabled + actually renderable).
  const enabledRendered = grid.filter((e) => e.enabled && rendered.has(e.id));
  // Palette candidates: renderable widgets that aren't on the grid.
  const paletteItems = grid.filter((e) => !e.enabled && rendered.has(e.id));

  const allEmpty = enabledRendered.length === 0;
  if (allEmpty && !isEditing && !focusing) {
    return (
      <p className="hud-note" style={{ padding: '6px 2px' }}>
        Your dashboard is empty.{' '}
        <button
          type="button"
          className="hud-textbtn"
          onClick={() => setEditing(true)}
        >
          Edit layout
        </button>{' '}
        to add widgets.
      </p>
    );
  }

  // In focus mode, reproject the matching widgets into one centered column;
  // otherwise render the stored (enabled) grid. Pure view transform — the
  // saved `layout` is untouched.
  const displayEntries = focusing
    ? focusLayout(
        enabledRendered.filter((e) => widgetMatchesLens(e.id, activeLens)),
      )
    : enabledRendered;

  // View mode: fit each tile to its measured content, then pack the grid up
  // so nothing scrolls and no gaps remain. Edit/focus modes keep explicit
  // geometry (the owner is arranging, or the lens set its own single column).
  const laidOut =
    isEditing || focusing
      ? displayEntries
      : compactUp(
          displayEntries.map((e) => ({
            ...e,
            h:
              measured[e.id] != null
                ? fitRows(measured[e.id], boundsOf(e.id))
                : e.h,
          })),
        );

  const tiles = laidOut.map((entry) => {
    const w = rendered.get(entry.id);
    if (!w) return null;
    return (
      <GridTile
        key={entry.id}
        entry={entry}
        widget={w}
        isEditing={isEditing}
        snapOn={snapOn}
        bounds={boundsOf(entry.id)}
        showLifetime={lensEnabled && !(w.isRangeAware ?? false)}
        onRemove={() => removeWidget(entry.id)}
        onCycleSize={() => cycleSize(entry.id)}
        onPreview={previewGeometry}
        onCommit={commitGeometry}
        pxToCells={pxToCells}
        onMeasure={handleMeasure}
      />
    );
  });

  const gridEl = (
    <div
      className="hud-freegrid"
      data-snap={snapOn ? 'on' : 'off'}
      data-editing={isEditing ? 'true' : 'false'}
      data-focus={focusing ? 'true' : 'false'}
      ref={gridRef}
    >
      {tiles}
    </div>
  );

  return (
    <>
      {showLens && (
        <nav aria-label="Focus lens" className="hud-controls">
          <span
            className="ss-eyebrow"
            style={{ marginRight: 6, color: 'var(--fg-dim)' }}
          >
            Focus
          </span>
          {LENSES.map((l) => {
            const active = l.id === activeLens;
            return (
              <button
                key={l.id}
                type="button"
                aria-pressed={active}
                onClick={() => setActiveLens(l.id)}
                className="hud-chip"
              >
                {l.label}
              </button>
            );
          })}
        </nav>
      )}

      {isEditing && (
        <div className="hud-controls" role="group" aria-label="Grid controls">
          <button
            type="button"
            className="hud-chip"
            aria-pressed={snapOn}
            onClick={() => setSnapOn((v) => !v)}
            title="Toggle snap-to-grid. On: tiles snap to every 2nd column. Off: free positioning with twice the anchor points."
          >
            {snapOn ? '⊞ Snap: on' : '⊟ Snap: off'}
          </button>
          <span className="hud-note" style={{ marginLeft: 4 }}>
            Drag a tile by its grip; drag the corner to resize; × removes it.
          </span>
        </div>
      )}

      {isEditing ? (
        <DndContext sensors={sensors} onDragEnd={onDragEnd}>
          {gridEl}
        </DndContext>
      ) : (
        gridEl
      )}

      {isEditing && (
        <WidgetPalette items={paletteItems} onAdd={addWidget} />
      )}
    </>
  );
}

/**
 * Edit-mode "Add widget" gallery. Lists every renderable widget not
 * currently on the dashboard with a short description; clicking (or
 * activating with the keyboard) adds it to a free cell. Native buttons →
 * keyboard-accessible for free.
 */
function WidgetPalette({
  items,
  onAdd,
}: {
  items: PositionedEntry[];
  onAdd: (id: string) => void;
}) {
  return (
    <section className="hud-palette" aria-label="Add widget">
      <h3 className="hud-palette__hd">Add widget</h3>
      {items.length === 0 ? (
        <p className="hud-palette__empty">
          Every widget is on your dashboard.
        </p>
      ) : (
        <ul className="hud-palette__grid">
          {items.map((entry) => {
            const meta = WIDGET_META[entry.id as WidgetId];
            const title = meta?.title ?? entry.id;
            return (
              <li key={entry.id} className="hud-palette__item">
                <span className="hud-palette__title">{title}</span>
                {meta?.description ? (
                  <span className="hud-palette__desc">{meta.description}</span>
                ) : null}
                <button
                  type="button"
                  className="hud-chip hud-chip--sm"
                  aria-label={`Add ${title}`}
                  onClick={() => onAdd(entry.id)}
                >
                  + Add
                </button>
              </li>
            );
          })}
        </ul>
      )}
    </section>
  );
}

interface GridTileProps {
  entry: PositionedEntry;
  widget: RenderedWidget;
  isEditing: boolean;
  snapOn: boolean;
  bounds: GridBounds;
  showLifetime: boolean;
  onRemove: () => void;
  onCycleSize: () => void;
  onPreview: (id: string, patch: Partial<PositionedEntry>) => void;
  onCommit: (id: string, patch: Partial<PositionedEntry>) => void;
  pxToCells: (dx: number, dy: number) => { dCols: number; dRows: number };
  /** Report this tile's natural content height (px) so the grid can fit its
   *  row-span to it. Fires on mount + whenever the content resizes. */
  onMeasure: (id: string, px: number) => void;
}

function GridTile({
  entry,
  widget,
  isEditing,
  snapOn,
  bounds,
  showLifetime,
  onRemove,
  onCycleSize,
  onPreview,
  onCommit,
  pxToCells,
  onMeasure,
}: GridTileProps) {
  const { attributes, listeners, setNodeRef, transform, isDragging } =
    useDraggable({ id: entry.id, disabled: !isEditing });

  // Combine dnd-kit's node ref with our own so we can measure the tile's
  // content. Measuring the content child (not the tile) gives the natural
  // height independent of the row-span we're about to set — no feedback loop.
  const sectionRef = useRef<HTMLElement | null>(null);
  const setRefs = useCallback(
    (el: HTMLElement | null) => {
      setNodeRef(el);
      sectionRef.current = el;
    },
    [setNodeRef],
  );
  useLayoutEffect(() => {
    const section = sectionRef.current;
    const body = section?.querySelector<HTMLElement>('.hud-tile__body');
    if (!body || body.children.length === 0) return;
    // Measure the FULL content extent (first child's top → last child's
    // bottom), not just the first child — widgets that render a fragment
    // (orgs, journey) have several top-level children. Using the extent
    // also captures inter-child gaps. Independent of the tile's row-span,
    // so setting the span from it can't loop.
    const report = () => {
      const kids = body.children;
      const top = kids[0].getBoundingClientRect().top;
      const bottom = kids[kids.length - 1].getBoundingClientRect().bottom;
      onMeasure(entry.id, bottom - top);
    };
    report();
    // ResizeObserver is absent in jsdom (unit tests) — one measure is enough
    // there; in the browser we keep observing so range switches / data loads
    // re-fit the tile.
    if (typeof ResizeObserver === 'undefined') return;
    const ro = new ResizeObserver(report);
    ro.observe(body);
    return () => ro.disconnect();
    // Re-run when the rendered body changes (range switch, data load).
  }, [entry.id, onMeasure, widget.body]);

  const gridStyle: React.CSSProperties = {
    // 1-based grid lines; span the tile's width/height in cells. `entry.h`
    // is the content-fitted span in view mode (see `laidOut`), the explicit
    // span in edit mode.
    gridColumn: `${entry.x + 1} / span ${entry.w}`,
    gridRow: `${entry.y + 1} / span ${entry.h}`,
    transform: CSS.Translate.toString(transform),
    opacity: isDragging ? 0.6 : 1,
    zIndex: isDragging ? 5 : undefined,
  };

  const eyebrowLabel = showLifetime ? (
    <>
      {widget.eyebrow}
      <span style={{ color: 'var(--fg-dim)' }}> · lifetime</span>
    </>
  ) : (
    widget.eyebrow
  );

  return (
    <Tile
      span={entry.w}
      live={widget.isRangeAware ?? false}
      eyebrow={eyebrowLabel}
      title={widget.title}
      nodeRef={setRefs}
      style={gridStyle}
      data={{
        'data-widget-id': entry.id,
        'data-widget-size': entry.size,
        'data-widget-enabled': String(entry.enabled),
      }}
      editChrome={
        isEditing ? (
          <TileChrome
            entry={entry}
            snapOn={snapOn}
            bounds={bounds}
            onRemove={onRemove}
            onCycleSize={onCycleSize}
            onCommit={onCommit}
            dragAttributes={
              attributes as unknown as React.HTMLAttributes<HTMLButtonElement>
            }
            dragListeners={
              listeners as unknown as React.DOMAttributes<HTMLButtonElement>
            }
          />
        ) : undefined
      }
      overlay={
        isEditing ? (
          <ResizeHandle
            entry={entry}
            snapOn={snapOn}
            bounds={bounds}
            onPreview={onPreview}
            onCommit={onCommit}
            pxToCells={pxToCells}
          />
        ) : undefined
      }
    >
      {widget.body}
    </Tile>
  );
}

/** Per-tile corner controls: drag grip, resize nudges (bounded by the
 *  widget's min/max envelope), size cycle, and remove. */
function TileChrome({
  entry,
  snapOn,
  bounds,
  onRemove,
  onCycleSize,
  onCommit,
  dragAttributes,
  dragListeners,
}: {
  entry: PositionedEntry;
  snapOn: boolean;
  bounds: GridBounds;
  onRemove: () => void;
  onCycleSize: () => void;
  onCommit: (id: string, patch: Partial<PositionedEntry>) => void;
  dragAttributes: React.HTMLAttributes<HTMLButtonElement>;
  dragListeners: React.DOMAttributes<HTMLButtonElement>;
}) {
  const step = snapStep(snapOn);
  const resizeW = (delta: number) =>
    onCommit(entry.id, { w: clampWidthTo(entry.w + delta, bounds) });
  const resizeH = (delta: number) =>
    onCommit(entry.id, { h: clampHeightTo(entry.h + delta, bounds) });

  return (
    <div role="toolbar" aria-label="Widget controls" style={{ display: 'flex', gap: 4 }}>
      <button
        type="button"
        className="hud-chip hud-chip--sm hud-chip--ghost hud-chip--grip"
        aria-label={`Drag ${entry.id} — pick up with space, arrow keys to move`}
        {...dragAttributes}
        {...dragListeners}
      >
        ⋮⋮
      </button>
      <button
        type="button"
        className="hud-chip hud-chip--sm hud-chip--ghost"
        aria-label="Narrower"
        disabled={entry.w <= bounds.minW}
        onClick={() => resizeW(-step)}
      >
        ◄
      </button>
      <button
        type="button"
        className="hud-chip hud-chip--sm hud-chip--ghost"
        aria-label="Wider"
        disabled={entry.w >= bounds.maxW || entry.x + entry.w >= GRID_COLS}
        onClick={() => resizeW(step)}
      >
        ►
      </button>
      <button
        type="button"
        className="hud-chip hud-chip--sm hud-chip--ghost"
        aria-label="Shorter"
        disabled={entry.h <= bounds.minH}
        onClick={() => resizeH(-step)}
      >
        ▲
      </button>
      <button
        type="button"
        className="hud-chip hud-chip--sm hud-chip--ghost"
        aria-label="Taller"
        disabled={entry.h >= bounds.maxH}
        onClick={() => resizeH(step)}
      >
        ▼
      </button>
      <button
        type="button"
        className="hud-chip hud-chip--sm hud-chip--ghost"
        aria-label={`Size: ${entry.size}`}
        onClick={onCycleSize}
      >
        ⤢
      </button>
      <button
        type="button"
        className="hud-chip hud-chip--sm hud-chip--ghost"
        aria-label={`Remove ${entry.id} from dashboard`}
        onClick={onRemove}
      >
        ✕
      </button>
    </div>
  );
}

/** Bottom-right pointer resize handle, clamped to the widget's envelope.
 *  Keyboard resize lives in the chrome buttons; this is the pointer
 *  affordance. */
function ResizeHandle({
  entry,
  snapOn,
  bounds,
  onPreview,
  onCommit,
  pxToCells,
}: {
  entry: PositionedEntry;
  snapOn: boolean;
  bounds: GridBounds;
  onPreview: (id: string, patch: Partial<PositionedEntry>) => void;
  onCommit: (id: string, patch: Partial<PositionedEntry>) => void;
  pxToCells: (dx: number, dy: number) => { dCols: number; dRows: number };
}) {
  const start = useRef<{ x: number; y: number; w: number; h: number } | null>(
    null,
  );

  const onPointerDown = (e: React.PointerEvent<HTMLDivElement>) => {
    e.preventDefault();
    e.stopPropagation();
    (e.target as HTMLElement).setPointerCapture(e.pointerId);
    start.current = { x: e.clientX, y: e.clientY, w: entry.w, h: entry.h };
  };

  const onPointerMove = (e: React.PointerEvent<HTMLDivElement>) => {
    if (!start.current) return;
    const { dCols, dRows } = pxToCells(
      e.clientX - start.current.x,
      e.clientY - start.current.y,
    );
    // Clamp to the widget envelope AND the remaining grid width at this x.
    const maxW = Math.min(bounds.maxW, GRID_COLS - entry.x);
    const w = clampWidthTo(snapValue(start.current.w + dCols, snapOn, bounds.minW, maxW), {
      ...bounds,
      maxW,
    });
    const h = clampHeightTo(
      snapValue(start.current.h + dRows, snapOn, bounds.minH, bounds.maxH),
      bounds,
    );
    onPreview(entry.id, { w, h });
  };

  const onPointerUp = (e: React.PointerEvent<HTMLDivElement>) => {
    if (!start.current) return;
    (e.target as HTMLElement).releasePointerCapture(e.pointerId);
    start.current = null;
    onCommit(entry.id, { w: entry.w, h: entry.h });
  };

  return (
    <div
      className="hud-freegrid__resize"
      role="presentation"
      aria-hidden="true"
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
    />
  );
}
