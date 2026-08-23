'use client';

import React from 'react';

/**
 * Bottom lens selector — the projection's primary navigation.
 *
 * The shipped lens set is All / Activity / Travel / Combat / Loadout /
 * Commerce (`lib/lens.ts`). A lens HIDES elements rather than changing them, so
 * a figure never disagrees with itself between lenses. Six is about the limit
 * for the rail's 32px gaps.
 *
 * Lenses are also reachable by number keys; Esc walks one depth out. Wire both.
 */
export interface LensItem {
  id?: string;
  name: string;
}

export function LensRail({
  lenses = [],
  active = -1,
  onSelect,
}: {
  lenses?: LensItem[];
  active?: number;
  onSelect?: (index: number) => void;
}) {
  return (
    <nav className="hp-lens" aria-label="Lenses">
      {lenses.map((l, i) => (
        <button
          key={l.id ?? i}
          type="button"
          aria-pressed={i === active}
          onClick={() => onSelect && onSelect(i)}
        >
          {l.name}
        </button>
      ))}
    </nav>
  );
}

export interface CrumbPart {
  t: React.ReactNode;
  /** Omit on the current (last) step — it is where you already are. */
  onClick?: () => void;
}

/**
 * Depth chain: Overview → lens → record. Earlier steps are clickable.
 *
 * `heading` renders the LAST part as an `<h1>`. The projection has no page
 * title of its own — the volume is the page — so without this a ported screen
 * ships with no h1 at all, which every flat screen it replaced had. The final
 * crumb step IS the page's name, so it is the honest place for it rather than
 * a hidden heading bolted on elsewhere. Styling is unchanged; only the tag is.
 */
export function Crumb({
  parts = [],
  heading = false,
}: {
  parts?: CrumbPart[];
  heading?: boolean;
}) {
  const last = parts.length - 1;
  return (
    <div className="hp-crumb">
      {parts.map((p, i) => (
        <React.Fragment key={i}>
          {i ? <s /> : null}
          {p.onClick ? (
            <button type="button" onClick={p.onClick}>
              {p.t}
            </button>
          ) : heading && i === last ? (
            <h1>{p.t}</h1>
          ) : (
            <span>{p.t}</span>
          )}
        </React.Fragment>
      ))}
    </div>
  );
}

/**
 * Range tabs — an underline that lights, never a filled chip.
 *
 * The set and the default are the product's (`lib/range.ts`): 24h / 7d / 30d /
 * 90d / All, defaulting to 7d. "All" means **365 days**, because that is the
 * hard retention limit — the product's own guide calls naming it "all time" a
 * lie by one word, so the qualifier ships with the control.
 *
 * PORT NOTE (gap A6): on /me the active range comes from the `?range=` URL
 * param, not component state, so server components re-query on a change and the
 * view stays shareable and back-button correct. Pass `renderItem` to render
 * each tab as a Next <Link> instead of a button.
 */
export function RangeTabs({
  ranges = ['24h', '7d', '30d', '90d', 'all'],
  active = '7d',
  onSelect,
  note = true,
  renderItem,
}: {
  ranges?: string[];
  active?: string;
  onSelect?: (range: string) => void;
  /** Shows the "All = 365 days" qualifier. Keep it visible. */
  note?: boolean;
  /** Renders each tab as something other than a button — e.g. a Next <Link>. */
  renderItem?: (
    range: string,
    label: string,
    isActive: boolean,
  ) => React.ReactNode;
}) {
  return (
    <div className="hp-rng">
      {ranges.map((r) => {
        const label = r === 'all' ? 'All' : r;
        const isActive = r === active;
        if (renderItem) {
          return (
            <React.Fragment key={r}>
              {renderItem(r, label, isActive)}
            </React.Fragment>
          );
        }
        return (
          <button
            key={r}
            type="button"
            aria-pressed={isActive}
            onClick={() => onSelect && onSelect(r)}
          >
            {label}
          </button>
        );
      })}
      {note ? <span className="rq">All = 365 days</span> : null}
    </div>
  );
}
