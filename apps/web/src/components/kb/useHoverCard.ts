'use client';

/**
 * The hover-card controller, extracted from `EntityLink`.
 *
 * `EntityLink` owned ~90 lines of open/measure/track/dismiss machinery for the
 * entity hover card. It was the only thing that could show one, which meant a
 * surface that cannot nest an anchor could not have hover cards at all — and
 * the projection's ranked rows are exactly that case: `MeterRow` makes the
 * WHOLE ROW the link (its own comment records that an anchor around the label
 * and a stretched overlay were both tried and both failed), so an `EntityLink`
 * inside one would nest anchors.
 *
 * Nothing here is new behaviour. It is the same measurement, the same
 * viewport clamping, the same capture-phase scroll tracking and the same
 * Escape handling, lifted so a second trigger shape can reuse it.
 */

import {
  useCallback,
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
} from 'react';
import { HOVER_CARD_WIDTH, type HoverCardPos } from './EntityHoverCard';
import { useIsClient } from '@/lib/use-is-client';

/** Gap between the trigger and the card. */
const GAP = 6;
/** Minimum distance kept from every viewport edge. */
const PAD = 8;

export interface HoverCardController<
  A extends HTMLElement,
  C extends HTMLElement,
> {
  hovered: boolean;
  setHovered: (v: boolean) => void;
  /** Stable id, for the trigger's `aria-describedby`. */
  cardId: string;
  anchorRef: React.RefObject<A | null>;
  cardRef: React.RefObject<C | null>;
  /** False until the first client render, so the portal never breaks hydration. */
  mounted: boolean;
}

export function useHoverCard<
  A extends HTMLElement = HTMLElement,
  C extends HTMLElement = HTMLElement,
>(): HoverCardController<A, C> {
  const [hovered, setHovered] = useState(false);
  // Stable id to wire the trigger's `aria-describedby` to the hover card so
  // screen-reader users get the detail on focus, not just sighted hover (M-W10).
  const cardId = useId();
  const anchorRef = useRef<A>(null);
  const cardRef = useRef<C>(null);

  // Portal target only exists on the client. Gate on a mounted flag rather
  // than `typeof document`, so the server render and the first client render
  // agree and hydration doesn't mismatch.
  const mounted = useIsClient();

  // `place` only MEASURES. It used to write the position itself, which made
  // every caller — the layout effect included — a state write routed through a
  // callback, and react-hooks/set-state-in-effect flags exactly that. Returning
  // the position lets the layout effect set it directly, which the rule allows
  // (measure-then-position is what useLayoutEffect is for), and the listeners
  // do the same.
  const place = useCallback((): HoverCardPos | null => {
    const anchor = anchorRef.current;
    const card = cardRef.current;
    if (!anchor || !card) return null;
    const a = anchor.getBoundingClientRect();
    const c = card.getBoundingClientRect();
    const vw = document.documentElement.clientWidth;
    const vh = document.documentElement.clientHeight;
    const width = c.width || HOVER_CARD_WIDTH;

    // Left-align with the trigger, then pull back inside the viewport.
    // Clamping the low end last matters: on a narrow screen the card can be
    // wider than the space available, and pinning the left edge beats losing
    // the start of every value.
    let left = a.left;
    left = Math.min(left, vw - PAD - width);
    left = Math.max(PAD, left);

    // Prefer below the trigger; flip above when there isn't room, which is the
    // common case for rows at the bottom of a scrolled pane or the page.
    let top = a.bottom + GAP;
    if (top + c.height > vh - PAD) {
      top = Math.max(PAD, a.top - c.height - GAP);
    }

    return { top: Math.round(top), left: Math.round(left) };
  }, []);

  // THE POSITION IS WRITTEN TO THE ELEMENT, NOT TO STATE.
  //
  // Measuring the DOM and storing the result in state made every open a
  // second render — commit, measure in a layout effect, set state, commit
  // again — and react-hooks/set-state-in-effect flags precisely that. The
  // geometry lives outside React; writing it straight back to the element's
  // style inside the layout effect lands before the same paint, with no state
  // and no second commit. Hidden until measured so nothing flashes at 0,0.
  const apply = useCallback(
    (p: HoverCardPos | null) => {
      const card = cardRef.current;
      if (!card) return;
      if (p) {
        card.style.top = `${p.top}px`;
        card.style.left = `${p.left}px`;
        card.style.visibility = 'visible';
      } else {
        card.style.visibility = 'hidden';
      }
    },
    [],
  );

  // Measure before paint so the card never shows at a stale position.
  useLayoutEffect(() => {
    apply(hovered ? place() : null);
  }, [hovered, place, apply]);

  // `position: fixed` does not follow the trigger, so track it while open.
  // Capture phase catches scrolls in an inner scroll container, not just the
  // window.
  useEffect(() => {
    if (!hovered) return;
    const onMove = () => apply(place());
    window.addEventListener('scroll', onMove, true);
    window.addEventListener('resize', onMove);
    return () => {
      window.removeEventListener('scroll', onMove, true);
      window.removeEventListener('resize', onMove);
    };
  }, [hovered, place]);

  // Escape dismisses the card without moving the pointer (WCAG 1.4.13,
  // content-on-hover-or-focus dismissable) — M-W10. Listened for on the
  // document: a pointer hover never gives the trigger focus, so a handler on
  // the wrapper would only ever fire for keyboard users.
  useEffect(() => {
    if (!hovered) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setHovered(false);
    };
    document.addEventListener('keydown', onKey);
    return () => document.removeEventListener('keydown', onKey);
  }, [hovered]);

  return { hovered, setHovered, cardId, anchorRef, cardRef, mounted };
}
