import React from 'react';
import Link from 'next/link';
import type { Route } from 'next';

/**
 * Widget archetype renderers — the shared module the whole dashboard is
 * built on. Every widget body composes ONE of these instead of hand-rolling
 * `hud-readout-*` markup, so the three cross-cutting rules live in exactly
 * one place each:
 *
 *   1. NEVER SCROLL for data — `RankedList` caps to a top-N and surfaces the
 *      remainder as a "See more →" link / "+N more" note, never a scrollbar.
 *   2. NO WASTED SPACE — bodies render only their bounded summary; the tile
 *      height is sized to fit it (see WIDGET_META bounds).
 *   3. CONSISTENT STYLE — one HUD readout/row/meter vocabulary, so a design
 *      tweak lands everywhere at once.
 *
 * These are pure presentational components (no data fetching, no hooks) so
 * they render inside the server-rendered widget bodies unchanged.
 */

// ── Readout group: a set of labelled numbers (the dominant archetype) ──────
export interface Readout {
  /** Small-caps caption. */
  label: React.ReactNode;
  /** The value (already formatted). */
  value: React.ReactNode;
  /** Optional trailing element after the label, e.g. an <InfoTip/>. */
  info?: React.ReactNode;
  /** Mark as secondary → collapses when the tile is squeezed (container query). */
  secondary?: boolean;
}

export function ReadoutGroup({
  readouts,
  note,
  layout = 'wrap',
}: {
  readouts: Readout[];
  note?: React.ReactNode;
  /** `wrap` = inline flex-wrap (default); `stack` = one per line. */
  layout?: 'wrap' | 'stack';
}) {
  return (
    <div>
      <div className={layout === 'stack' ? 'hud-readout-stack' : 'hud-readout-wrap'}>
        {readouts.map((r, i) => (
          <span
            key={i}
            className={r.secondary ? 'hud-readout hud-secondary' : 'hud-readout'}
          >
            <span className="k">
              {r.label}
              {r.info}
            </span>
            {r.value}
          </span>
        ))}
      </div>
      {note != null && <p className="hud-note">{note}</p>}
    </div>
  );
}

// ── Ranked list: top-N rows + count, with cap + "See more" baked in ────────
export interface Row {
  key: string;
  label: React.ReactNode;
  value: React.ReactNode;
}

export interface SeeMore {
  href: Route;
  /** Builds the link text from the hidden count + total. */
  label: (hidden: number, total: number) => React.ReactNode;
}

export function RankedList({
  rows,
  cap,
  seeMore,
  note,
}: {
  rows: Row[];
  /** Max rows shown; the rest go behind `seeMore`. Omit = show all (caller
   *  guarantees the list is already bounded). */
  cap?: number;
  seeMore?: SeeMore;
  note?: React.ReactNode;
}) {
  const shown = typeof cap === 'number' ? rows.slice(0, cap) : rows;
  const hidden = rows.length - shown.length;
  const link =
    hidden > 0 && seeMore ? (
      <Link href={seeMore.href}>{seeMore.label(hidden, rows.length)}</Link>
    ) : null;
  return (
    <div>
      <ul className="hud-readout-list">
        {shown.map((r) => (
          <li key={r.key} className="hud-readout-row">
            <span className="hud-trunc">{r.label}</span>
            <span className="hud-readout">{r.value}</span>
          </li>
        ))}
      </ul>
      {(note != null || link) && (
        <p className="hud-note">
          {note}
          {note != null && link ? ' · ' : null}
          {link}
        </p>
      )}
    </div>
  );
}

// ── Meter list: labelled share-of-total bars ───────────────────────────────
export interface Meter {
  label: React.ReactNode;
  value: React.ReactNode;
  /** 0–100. */
  pct: number;
}

export function MeterList({
  header,
  meters,
  note,
}: {
  header?: Readout[];
  meters: Meter[];
  note?: React.ReactNode;
}) {
  return (
    <div className="hud-readout-stack">
      {header && header.length > 0 && (
        <div className="hud-readout-wrap">
          {header.map((r, i) => (
            <span key={i} className="hud-readout">
              <span className="k">{r.label}</span>
              {r.value}
            </span>
          ))}
        </div>
      )}
      <ul className="hud-readout-list" style={{ listStyle: 'none', margin: 0, padding: 0 }}>
        {meters.map((m, i) => (
          <li key={i} className="hud-meter-row">
            <span className="k">{m.label}</span>
            <span className="hud-meter">
              <span
                className="hud-meter__fill"
                style={{ ['--val' as string]: `${Math.max(0, Math.min(100, m.pct))}%` } as React.CSSProperties}
              />
            </span>
            <span className="hud-readout">{m.value}</span>
          </li>
        ))}
      </ul>
      {note != null && <p className="hud-note">{note}</p>}
    </div>
  );
}
