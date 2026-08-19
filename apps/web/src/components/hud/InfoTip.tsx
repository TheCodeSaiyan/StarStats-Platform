'use client';

import React, { useCallback, useEffect, useId, useLayoutEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';

/**
 * Accessible "how was this calculated?" affordance: a small [i] button that
 * reveals a short explanation on hover, keyboard focus, or tap.
 *
 * Because several /me metrics are INFERRED from log lines (e.g. "quantum
 * jumps" counted from target-selection events), the honest thing is to tell
 * the user how the number was derived. The explanation is wired via
 * `aria-describedby` at all times, so screen-reader users get it on focus
 * even though the popover is only visually revealed on hover/focus/tap.
 *
 * WHY THE POPOVER IS PORTALED
 * ---------------------------
 * It used to be an absolutely-positioned child of the button, and inside a
 * widget tile it was invisible: it opened correctly but every one of its
 * ancestors clipped it. Measured on /me at v1.8.170 — popover at y=[177,290]
 * against `.hud-tile__body` [293,336] (overflow-y:auto), `.hud-tile`
 * [266,344] (overflow:hidden) and `.ss-main` (overflow:auto). It rendered
 * entirely above its own tile.
 *
 * Opening those containers is NOT the fix. `.hud-tile__body` keeps
 * `overflow-y: auto` as a deliberate safety net — hud.css records that
 * clipping silently swallowed travel's routes+map in v1.8.107, and a
 * scrollbar is the lesser evil to losing data. So the popover leaves the
 * container instead: it renders into `document.body` with `position: fixed`
 * and coordinates measured from the button.
 *
 * The cost of `fixed` is that the popover no longer travels with its anchor,
 * so an open tip is repositioned on scroll and resize.
 */
export interface InfoTipProps {
  /** The explanation shown in the popover. */
  text: string;
  /** Metric name, folded into the button's accessible label. */
  label?: string;
}

/** Gap between the [i] button and the popover edge. */
const GAP = 6;
/** Minimum distance the popover keeps from any viewport edge. */
const PAD = 8;

interface Pos {
  top: number;
  left: number;
}

export function InfoTip({ text, label }: InfoTipProps) {
  const [open, setOpen] = useState(false);
  const [pos, setPos] = useState<Pos | null>(null);
  const id = useId();
  const btnRef = useRef<HTMLButtonElement>(null);
  const popRef = useRef<HTMLSpanElement>(null);

  // Portal target only exists on the client. Gate on a mounted flag rather
  // than `typeof document`, so the server render and the first client render
  // agree and hydration doesn't mismatch.
  const [mounted, setMounted] = useState(false);
  useEffect(() => setMounted(true), []);

  const place = useCallback(() => {
    const btn = btnRef.current;
    const pop = popRef.current;
    if (!btn || !pop) return;
    const b = btn.getBoundingClientRect();
    const p = pop.getBoundingClientRect();
    const vw = document.documentElement.clientWidth;
    const vh = document.documentElement.clientHeight;

    // Centre on the button, then pull back inside the viewport. Clamping the
    // low end last matters: on a narrow screen the popover can be wider than
    // the space available, and pinning the left edge beats centring it.
    let left = b.left + b.width / 2 - p.width / 2;
    left = Math.min(left, vw - PAD - p.width);
    left = Math.max(PAD, left);

    // Prefer above (the readout it explains is usually below it); flip under
    // the button when there isn't room, which is the common case for tiles
    // near the top of the grid.
    let top = b.top - p.height - GAP;
    if (top < PAD) top = Math.min(b.bottom + GAP, vh - PAD - p.height);

    setPos({ top: Math.round(top), left: Math.round(left) });
  }, []);

  // Measure before paint so the popover never shows at a stale position.
  useLayoutEffect(() => {
    if (!open) {
      setPos(null);
      return;
    }
    place();
  }, [open, place]);

  // `position: fixed` does not follow the anchor, so track it while open.
  // Capture phase catches scrolls in the tile body and `.ss-main`, not just
  // the window.
  useEffect(() => {
    if (!open) return;
    const onMove = () => place();
    window.addEventListener('scroll', onMove, true);
    window.addEventListener('resize', onMove);
    return () => {
      window.removeEventListener('scroll', onMove, true);
      window.removeEventListener('resize', onMove);
    };
  }, [open, place]);

  // Esc closes; an outside tap/click closes (touch users who opened by tap).
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setOpen(false);
    };
    const onDocPointer = (e: Event) => {
      const t = e.target as Node;
      // The popover is portaled, so it is NOT inside the button's subtree —
      // it needs its own containment check or clicking the explanation
      // (to select text) would dismiss it.
      if (btnRef.current?.contains(t) || popRef.current?.contains(t)) return;
      setOpen(false);
    };
    document.addEventListener('keydown', onKey);
    document.addEventListener('click', onDocPointer);
    return () => {
      document.removeEventListener('keydown', onKey);
      document.removeEventListener('click', onDocPointer);
    };
  }, [open]);

  const pop = (
    <span
      ref={popRef}
      id={id}
      role="tooltip"
      className={`infotip__pop${open && pos ? ' infotip__pop--open' : ''}`}
      style={pos ? { top: pos.top, left: pos.left } : undefined}
      // Keep it open while the pointer is over the explanation itself, so it
      // can be read and selected without racing the button's mouseleave.
      onMouseEnter={() => setOpen(true)}
      onMouseLeave={() => setOpen(false)}
    >
      {text}
    </span>
  );

  return (
    <span className="infotip">
      <button
        ref={btnRef}
        type="button"
        className="infotip__btn"
        aria-label={label ? `How ${label} is calculated` : 'How this is calculated'}
        aria-expanded={open}
        aria-describedby={id}
        onMouseEnter={() => setOpen(true)}
        onMouseLeave={() => setOpen(false)}
        onFocus={() => setOpen(true)}
        onBlur={() => setOpen(false)}
        onClick={(e) => {
          // A click also focuses (which opens), so this must OPEN, not
          // toggle — otherwise focus-open + click-toggle nets closed.
          // Dismissal is via mouse-leave, Escape, or an outside tap.
          e.stopPropagation();
          setOpen(true);
        }}
      >
        <span aria-hidden="true">i</span>
      </button>
      {/* Rendered even while closed: `aria-describedby` must resolve to a
          live node so assistive tech reads the explanation on focus. */}
      {mounted ? createPortal(pop, document.body) : pop}
    </span>
  );
}
