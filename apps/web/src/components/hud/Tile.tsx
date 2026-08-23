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

/**
 * The shell every widget renders into.
 *
 * REDRAWN AS A PLANE. This was the single highest-leverage flat holdover in the
 * product: one component, and every widget tile on every surface that renders a
 * canvas draws through it. Squaring its corners from a bridge rule is what made
 * those pages "the old one in a new box"; this emits the projection's own
 * markup instead — a lit hairline sheet with a tracked caption, the substat as
 * the caption's trailing affordance, and the empty state as a dim line rather
 * than a box.
 *
 * WHAT DELIBERATELY DID NOT CHANGE, because all of it is load-bearing:
 *
 *   - The `<section>` element, `nodeRef`, `data-*` and `data-title`. dnd-kit
 *     sets the ref, the layout editor reads the attributes, and
 *     `widget-audit.spec.ts` measures tiles through them.
 *   - `hud-tile__title` on the title. The mobile type rules in
 *     starstats-tokens.css scope `h1/h2` shrinking with `:not(.hud-tile__title)`
 *     — that exclusion is what stops HUD titles being blown up on phones, and
 *     dropping the class would silently undo it.
 *   - `hud-tile__body` and its `overflow-y: auto`. hud.css records that removing
 *     that clipping swallowed travel's routes and map in v1.8.107; it is a
 *     safety net, not decoration.
 *   - `editChrome` and `overlay` as direct children of the section, so the
 *     resize handle anchors to the tile rather than scrolling with its body.
 */
export function Tile({ span = 1, eyebrow, title, substat, empty, editChrome, style, nodeRef, data, live = true, overlay, children }: TileProps) {
  const sectionStyle: React.CSSProperties = { gridColumn: `span ${span}`, ...style };
  const cls = [
    'hud-tile',
    'hp-plane',
    'flat',
    live ? 'hud-tile--live' : '',
  ]
    .filter(Boolean)
    .join(' ');
  return (
    <section ref={nodeRef} className={cls} style={sectionStyle} data-title={title} {...data}>
      {editChrome ? <div className="hud-tile__chrome">{editChrome}</div> : null}
      <header className="cap hud-tile__hd">
        {eyebrow ? <span className="hud-tile__eyebrow">{eyebrow}</span> : null}
        <span className="hud-tile__title">{title}</span>
        {substat ? <span className="tr hud-tile__sub">{substat}</span> : null}
      </header>
      {empty ? (
        <p className="hp-empty hud-tile__empty">{empty}</p>
      ) : (
        <div className="hud-tile__body">{children}</div>
      )}
      {overlay ?? null}
    </section>
  );
}
