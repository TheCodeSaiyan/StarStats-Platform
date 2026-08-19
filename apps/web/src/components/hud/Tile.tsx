import React from 'react';

export interface TileProps {
  /** Grid column span (1–4). */
  span?: number;
  /** Uppercase micro-label above the title (category). Accepts ReactNode for inline decorations. */
  eyebrow?: React.ReactNode;
  /** Tile title. */
  title: string;
  /** Right-aligned sub-stat in the header (e.g. "22 jumps · 14 hops"). */
  substat?: React.ReactNode;
  /** When set, the tile renders this muted one-liner instead of the body
   *  (compact empty/lifetime state — never a big box). */
  empty?: string | null;
  /** Edit-mode corner chrome (drag grip + resize/hide), rendered top-right. */
  editChrome?: React.ReactNode;
  /** Extra inline styles applied to the <section> (e.g. dnd transform/transition). */
  style?: React.CSSProperties;
  /** Ref callback for the <section> element (used by dnd-kit's setNodeRef). */
  nodeRef?: (el: HTMLElement | null) => void;
  /** Arbitrary data-* attributes spread onto the <section> element. */
  data?: Record<string, string | number | boolean>;
  /** Whether this pane shows live/streaming telemetry — earns the corner
   *  brackets (bridge). Defaults true (the widget canvas). Static panes
   *  pass live={false} to render as a plain seam. */
  live?: boolean;
  /** Absolutely-positioned overlay rendered as a DIRECT child of the
   *  <section> (not inside the scrollable body) — e.g. the free-grid
   *  resize handle, which must anchor to the tile, not scroll with it. */
  overlay?: React.ReactNode;
  children?: React.ReactNode;
}

/** Dense HUD tile shell. Sizes to content (no fixed height). */
export function Tile({ span = 1, eyebrow, title, substat, empty, editChrome, style, nodeRef, data, live = true, overlay, children }: TileProps) {
  const sectionStyle: React.CSSProperties = { gridColumn: `span ${span}`, ...style };
  return (
    <section ref={nodeRef} className={live ? 'hud-tile hud-tile--live' : 'hud-tile'} style={sectionStyle} data-title={title} {...data}>
      {editChrome ? <div className="hud-tile__chrome">{editChrome}</div> : null}
      <header className="hud-tile__hd">
        {eyebrow ? <span className="hud-tile__eyebrow">{eyebrow}</span> : null}
        <span className="hud-tile__title">{title}</span>
        {substat ? <span className="hud-tile__sub">{substat}</span> : null}
      </header>
      {empty ? <p className="hud-tile__empty">{empty}</p> : <div className="hud-tile__body">{children}</div>}
      {overlay ?? null}
    </section>
  );
}
