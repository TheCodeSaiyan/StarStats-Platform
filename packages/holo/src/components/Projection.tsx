'use client';

import React from 'react';

/**
 * The projection volume. Owns the emitter, the depth layers (cursor parallax),
 * the hex field and floor rings, the scanline/flicker texture, and the
 * recalibration shock + wipe. Everything else in the system floats inside it.
 *
 * `mode` drives the three depths — overview → detail → inspect — by shifting
 * the ring and core left and revealing the matching pane (patterns-holo.css).
 *
 * The stage supplies `perspective: 1500px`, which `Plane`'s tilt REQUIRES. A
 * plane rendered outside a Projection needs its own perspective ancestor or it
 * draws flat.
 *
 * `data-cal` on this element is what scopes the whole beam token layer — the
 * projection's tokens are deliberately NOT on `:root` while the flat
 * `design-tokens` system coexists (see styles/index.css).
 */
export type Calibration = 'terra' | 'stanton' | 'pyro' | 'nyx';
export type ProjectionMode = 'overview' | 'detail' | 'inspect';
export type ProjectionSurface = 'brand' | 'console';

export interface ProjectionProps
  extends Omit<React.HTMLAttributes<HTMLDivElement>, 'children'> {
  mode?: ProjectionMode;
  calibration?: Calibration;
  /** Shows the inline × on every removable element. */
  editing?: boolean;
  /**
   * Recalibration is an EVENT, not a repaint. Bump this to fire the shock ring,
   * scan wipe, emitter surge and the core's chromatic jolt. Passing the same
   * value does nothing.
   */
  recalKey?: number;
  chrome?: React.ReactNode;
  crumb?: React.ReactNode;
  lens?: React.ReactNode;
  hint?: React.ReactNode;
  overlay?: React.ReactNode;
  parallax?: boolean;
  /**
   * Declared intent, never inferred from content. `brand` opens the ring up for
   * a wordmark; `console` kills the ambience entirely (no parallax, scanlines
   * or floor) because at eight hours a day it is noise.
   */
  surface?: ProjectionSurface;
  children?: React.ReactNode;
}

export function Projection({
  mode = 'overview',
  calibration = 'terra',
  editing = false,
  recalKey = 0,
  chrome,
  crumb,
  lens,
  hint,
  overlay,
  parallax = true,
  surface,
  children,
  style,
  ...rest
}: ProjectionProps) {
  const ref = React.useRef<HTMLDivElement | null>(null);
  const [recal, setRecal] = React.useState(false);
  /* Parallax is a cursor affordance. On touch there is nothing to track, so
   * the layers stay put rather than jumping on first tap. */
  const [coarse, setCoarse] = React.useState(false);
  React.useEffect(() => {
    if (typeof window === 'undefined' || !window.matchMedia) return;
    const mq = window.matchMedia('(pointer: coarse)');
    const sync = () => setCoarse(mq.matches);
    sync();
    mq.addEventListener?.('change', sync);
    return () => mq.removeEventListener?.('change', sync);
  }, []);
  /* Console surfaces never parallax — ambience is off by definition. */
  const live = parallax && !coarse && surface !== 'console';

  React.useEffect(() => {
    if (!recalKey) return;
    setRecal(false);
    const id = requestAnimationFrame(() => setRecal(true));
    const t = setTimeout(() => setRecal(false), 760);
    return () => {
      cancelAnimationFrame(id);
      clearTimeout(t);
    };
  }, [recalKey]);

  const onMove = (e: React.MouseEvent<HTMLDivElement>) => {
    if (!live) return;
    const stage = ref.current;
    if (!stage) return;
    const r = stage.getBoundingClientRect();
    const dx = (e.clientX - r.left) / r.width - 0.5;
    const dy = (e.clientY - r.top) / r.height - 0.5;
    stage.querySelectorAll<HTMLElement>('.hp-layer').forEach((l) => {
      const d = Number(l.dataset.depth || 0);
      l.style.transform = `translate3d(${-dx * d}px,${-dy * d * 0.6}px,0) rotateY(${-dx * 2.4}deg) rotateX(${dy * 1.7}deg)`;
    });
  };
  const onLeave = () => {
    ref.current?.querySelectorAll<HTMLElement>('.hp-layer').forEach((l) => {
      l.style.transform = '';
    });
  };

  return (
    <div
      ref={ref}
      className="hp-stage"
      data-mode={mode}
      data-cal={calibration}
      data-surface={surface}
      data-editing={editing ? 'true' : undefined}
      data-recal={recal ? '' : undefined}
      onMouseMove={live ? onMove : undefined}
      onMouseLeave={live ? onLeave : undefined}
      data-pointer={coarse ? 'coarse' : undefined}
      style={style}
      {...rest}
    >
      <a className="hp-skip" href="#hp-content">
        Skip to content
      </a>
      <div className="hp-emit" />
      <div className="hp-layer" data-depth="7">
        <div className="hp-hex" />
        <div className="hp-floor">
          <div />
          <div />
          <div />
          <i />
          <s />
        </div>
      </div>
      {/* THE MAIN LANDMARK, and it belongs here rather than on the surface
          root — an addition made during the port.

          Every ported shell used to put `role="main"` on its own outer wrapper,
          which contains the `ChromeBar`. That made the landmark include the
          nav: a screen-reader user jumping to main got the chrome with it, and
          a test that scoped a link assertion to `main` to EXCLUDE the nav
          silently stopped meaning anything (`support-help.spec.ts` hit exactly
          that). This element already wraps `children` alone — chrome, crumb and
          lens are siblings — and the skip link above already targets it, so it
          is the honest place for the landmark.

          `display: contents` is load-bearing (the children are placed by the
          stage's own layout, not by a box here) and the ARIA role survives it:
          verified in Chromium that the element still exposes exactly one `main`
          and that a nav sibling is outside it. */}
      <div id="hp-content" role="main" tabIndex={-1} style={{ display: 'contents' }}>
        {children}
      </div>
      {chrome}
      <div className="hp-shock" />
      <div className="hp-wipe" />
      {crumb}
      {/* One centred stack: the hint is a line beneath the rail, never a
          separately anchored element that can cross it. */}
      {lens || hint ? (
        <div className="hp-railstack">
          {lens}
          {hint ? <div className="hp-hint">{hint}</div> : null}
        </div>
      ) : null}
      {overlay}
    </div>
  );
}

/**
 * Depth layer. Wrap projection content so it takes cursor parallax.
 *
 * Depths are fixed BY ROLE — floor 7 (built in), ring 20, core 36,
 * callouts/panes 54. Don't invent new ones.
 */
export function Depth({
  depth = 20,
  children,
  ...rest
}: { depth?: number; children?: React.ReactNode } & React.HTMLAttributes<HTMLDivElement>) {
  return (
    <div className="hp-layer" data-depth={depth} {...rest}>
      {children}
    </div>
  );
}
