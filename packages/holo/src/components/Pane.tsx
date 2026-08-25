'use client';

import React from 'react';

/**
 * The pane: the docked reading surface at depth 2 (lens detail) and depth 3
 * (inspector). Header with a thin tracked title, optional context, a trailing
 * control slot (range tabs), then whatever the reader needs.
 *
 * `variant="static"` docks it into normal page flow — that's how the
 * non-projection screens (settings, directory, reference) are built.
 *
 * Panes scroll INTERNALLY with a sticky header (`max-height: calc(100% - 176px)`
 * in patterns-holo.css). Never let content stretch the volume: a stress run of
 * 220 log rows grew a pane to 6191px and it ran off the stage floor.
 */
export interface PaneProps {
  /** Depth slot. `inspect` gets the wider 614px box automatically. */
  pane?: 'detail' | 'inspect';
  title?: React.ReactNode;
  /** Sub-title context line, e.g. "Crusader · landing zone". */
  ctx?: React.ReactNode;
  /** Header control slot — the range tabs live here. */
  trailing?: React.ReactNode;
  /** `static` docks the pane into normal page flow. */
  variant?: 'static';
  /**
   * Heading level for `title`. Defaults to 2, which is right everywhere the
   * page already has an h1 above or inside it.
   *
   * `/u/[handle]` is the exception: the volume has no titled surface, so the
   * pane header IS the page heading and the handle in it is the h1. Passing an
   * `<h1>` as `title` instead nests one heading inside another and gives the
   * page two elements with the same accessible name — which is what happened
   * before this prop existed.
   */
  level?: 1 | 2;
  children?: React.ReactNode;
  style?: React.CSSProperties;
}

export function Pane({
  pane = 'detail',
  title,
  ctx,
  trailing,
  variant,
  level = 2,
  children,
  style,
}: PaneProps) {
  const H = (level === 1 ? 'h1' : 'h2') as 'h1' | 'h2';
  const cls = ['hp-pane', variant === 'static' ? 'hp-pane--static' : '']
    .filter(Boolean)
    .join(' ');
  return (
    <section className={cls} data-pane={pane} style={style}>
      <header className="hp-phd">
        <H>{title}</H>
        {ctx ? <span className="ctx">{ctx}</span> : null}
        {trailing}
      </header>
      {children}
    </section>
  );
}

export interface SubStatItem {
  /** Label. Kept short — the grid is four columns wide. */
  k: React.ReactNode;
  v: React.ReactNode;
  /** Unit, set small after the figure. */
  u?: React.ReactNode;
  /**
   * The supporting line under the figure — the DERIVATION, not decoration.
   *
   * An addition made while redrawing the knowledge base. Its headline metrics
   * carry a peer-relative band ("faster than 82% of light fighters"), and the
   * system's own rule is that an inferred or comparative figure states where it
   * came from rather than standing alone. Without somewhere to put it the
   * choice was to drop the claim or keep the flat card that held it.
   */
  sub?: React.ReactNode;
  /** `bad` joins the set the chips already carry, for a figure in the wrong
   *  tail of its distribution. */
  tone?: 'warn' | 'good' | 'bad';
}

/**
 * Is this value a FIGURE, or words?
 *
 * The slot is sized for figures — 25px, tabular, never wrapping — because
 * "21,909" must not break across lines. A shop name in that face cannot fit a
 * 194px column and gets truncated to "NoodleBar A Food Rest…", which is not
 * much better than the raw id it replaced.
 *
 * The system already draws this distinction one level down: a ranked row whose
 * value is words drops the meter and sets the value small
 * (`.hp-rw--text .vv`). This is the same rule for the same reason.
 *
 * A digit anywhere means figure — "4h 12m", "75%" and "21,909aUEC" are all
 * figures; "NoodleBar A Food RestStop" is not.
 */
function isFigure(v: React.ReactNode): boolean {
  if (typeof v === 'number') return true;
  if (typeof v !== 'string') return true; // a node is the caller's business
  return /\d/.test(v);
}

/**
 * Four hairline-divided figures under a pane header.
 *
 * EXACTLY four items — the grid is four columns and a fifth wraps badly.
 * Fewer than four is fine (a short row), more is not.
 */
export function SubStats({ items = [] }: { items?: SubStatItem[] }) {
  return (
    <div className="hp-subs">
      {items.map((it, i) => (
        <div key={i} data-kind={isFigure(it.v) ? undefined : 'text'}>
          <span>{it.k}</span>
          <b
            className={
              it.tone === 'warn'
                ? 'w'
                : it.tone === 'good'
                  ? 'g'
                  : it.tone === 'bad'
                    ? 'b'
                    : undefined
            }
          >
            {it.v}
            {it.u ? <small>{it.u}</small> : null}
          </b>
          {it.sub ? <i>{it.sub}</i> : null}
        </div>
      ))}
    </div>
  );
}
