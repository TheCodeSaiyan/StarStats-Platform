'use client';

import React from 'react';
import { createPortal } from 'react-dom';

/**
 * BeamTip — the projection's disclosure affordance (gap A4).
 *
 * WHY THIS EXISTS. The design system mandates that an inferred figure carries
 * its derivation ("reconstructed from Corpse lines, as the game no longer logs
 * deaths directly") and names `InfoTip` as the vehicle — but ships no tooltip,
 * popover or dialog, and lists Modal/Toast/Tabs/Select as deliberately absent.
 * That is an inconsistency in the system, not a gap in the port: the rule
 * cannot be honoured with the primitives provided. So this is a considered
 * addition rather than an invention — it borrows the flat product's `InfoTip`
 * behaviour and redraws it in the beam.
 *
 * IT PORTALS, AND THAT IS LOAD-BEARING. `.hp-pane` scrolls internally and
 * `.hp-plane` is a transformed sheet; either would clip an absolutely
 * positioned popover, and a `transform` ancestor also becomes the containing
 * block for `position: fixed` descendants. The same trap has produced two live
 * bugs in this product already (the tray drawer, and an InfoTip that opened
 * correctly for months while being invisible inside every widget tile). So the
 * popover renders into `document.body`, positions from the trigger's rect, and
 * the outside-click handler carries a SECOND containment check because a
 * portaled popover is no longer inside the trigger's subtree.
 *
 * ONLY FIGURES GLOW, so the trigger is a lit hairline under the value — never a
 * boxed icon, never a filled affordance, and never an emoji.
 */
export interface BeamTipProps {
  /** The figure the note is about. Rendered inline, unchanged. */
  children: React.ReactNode;
  /** The derivation. Plain text — this is a caption, so it never glows. */
  note: React.ReactNode;
  /**
   * Accessible name for the trigger. Defaults to a generic phrase; pass
   * something specific when the figure alone will not identify it.
   */
  label?: string;
}

/** Gap between the trigger and the popover, in px. */
const OFFSET = 8;
/** Popover width. Fixed so the note wraps predictably at any figure size. */
const WIDTH = 248;
/** Keep the popover this far from the viewport edge. */
const MARGIN = 12;

export function BeamTip({ children, note, label }: BeamTipProps) {
  const [open, setOpen] = React.useState(false);
  const [mounted, setMounted] = React.useState(false);
  const [pos, setPos] = React.useState<{ top: number; left: number } | null>(
    null,
  );
  // The popover portals to document.body, OUTSIDE the projection stage — and
  // the beam token layer is scoped to `[data-cal]` rather than `:root` so it
  // cannot leak onto the un-ported flat pages during the coexistence period.
  // So the popover has to carry its own calibration across the portal boundary;
  // inheriting is not available to it. Read from the trigger's nearest stage.
  const [cal, setCal] = React.useState<string | null>(null);
  const triggerRef = React.useRef<HTMLButtonElement | null>(null);
  const popRef = React.useRef<HTMLDivElement | null>(null);

  // Portals need a DOM target, which does not exist during SSR.
  React.useEffect(() => setMounted(true), []);

  const place = React.useCallback(() => {
    const el = triggerRef.current;
    if (!el) return;
    const r = el.getBoundingClientRect();
    // Centre on the trigger, then clamp inside the viewport so a callout near
    // the stage edge does not push the note off-screen.
    setCal(el.closest('[data-cal]')?.getAttribute('data-cal') ?? null);
    const raw = r.left + r.width / 2 - WIDTH / 2;
    const left = Math.max(
      MARGIN,
      Math.min(raw, window.innerWidth - WIDTH - MARGIN),
    );
    setPos({ top: r.bottom + OFFSET, left });
  }, []);

  React.useEffect(() => {
    if (!open) return;
    place();
    // The projection has no page scroll, but a Pane scrolls internally and the
    // window can still resize — both move the trigger out from under the note.
    // `capture` catches scrolls on the internal scroller, not just the window.
    const onScroll = () => place();
    window.addEventListener('scroll', onScroll, true);
    window.addEventListener('resize', onScroll);
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setOpen(false);
    };
    const onDown = (e: MouseEvent) => {
      const t = e.target as Node;
      // TWO checks: the portaled popover is not inside the trigger's subtree,
      // so testing only the trigger would close it on its own clicks.
      if (triggerRef.current?.contains(t)) return;
      if (popRef.current?.contains(t)) return;
      setOpen(false);
    };
    document.addEventListener('keydown', onKey);
    document.addEventListener('mousedown', onDown);
    return () => {
      window.removeEventListener('scroll', onScroll, true);
      window.removeEventListener('resize', onScroll);
      document.removeEventListener('keydown', onKey);
      document.removeEventListener('mousedown', onDown);
    };
  }, [open, place]);

  return (
    <span className="hp-tip">
      {children}
      <button
        ref={triggerRef}
        type="button"
        className="hp-tip__t"
        aria-expanded={open}
        aria-label={label ?? 'How this figure is derived'}
        onClick={() => setOpen((v) => !v)}
      >
        <span className="hp-tip__rule" aria-hidden="true" />
      </button>
      {mounted && open && pos
        ? createPortal(
            <div
              ref={popRef}
              role="tooltip"
              className="hp-tip__pop"
              data-cal={cal ?? undefined}
              style={{ top: pos.top, left: pos.left, width: WIDTH }}
            >
              {note}
            </div>,
            document.body,
          )
        : null}
    </span>
  );
}
