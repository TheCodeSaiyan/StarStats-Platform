'use client';

import React from 'react';

/**
 * The angled data plane — the system's card. Tilted 13° away from the reader
 * so it reads as a sheet standing in the volume, with two corner brackets and
 * a tracked caption. `flat` for stacked reading, `left` to mirror the tilt.
 *
 * PORT NOTE (gap A3): `trailing` and `empty` are additions to the upstream
 * component, not inventions of a new shape — they mirror what `Pane` already
 * does. The flat product gave every widget a frame with a title, a "See all"
 * link and a per-tile empty state (`WidgetCanvas` sets `empty: body == null`
 * and renders `<NoSignal compact />`); the projection had a caption and a hint
 * and nowhere to put the other two. `trailing` carries the real Next <Link>
 * out to the full page, `empty` carries the no-data state.
 */
export interface PlaneProps
  extends Omit<React.HTMLAttributes<HTMLDivElement>, 'title'> {
  /** Tracked uppercase caption. Omit for an uncaptioned sheet. */
  cap?: React.ReactNode;
  /** Right-aligned caption affordance, e.g. "select a row →". Text only. */
  hint?: React.ReactNode;
  /** Right-aligned interactive slot — the "see all →" link. (Addition.) */
  trailing?: React.ReactNode;
  /**
   * Rendered INSTEAD of `children` when there is no data. Pass the node, not
   * a boolean: a Plane with an `empty` and no children shows the empty state.
   * (Addition.)
   */
  empty?: React.ReactNode;
  /** `flat` for stacked reading and tables; `left` mirrors the tilt. */
  tilt?: 'right' | 'left' | 'flat';
  children?: React.ReactNode;
}

export function Plane({
  cap,
  hint,
  trailing,
  empty,
  tilt = 'right',
  style,
  children,
  ...rest
}: PlaneProps) {
  const cls = [
    'hp-plane',
    tilt === 'flat' ? 'flat' : '',
    tilt === 'left' ? 'left' : '',
  ]
    .filter(Boolean)
    .join(' ');
  // "No children" is the empty signal, so a caller can pass a mapped array
  // that came back empty without also threading a boolean.
  const hasChildren = React.Children.count(children) > 0;
  return (
    <div className={cls} style={style} {...rest}>
      {cap || trailing ? (
        <div className="cap">
          {cap}
          {hint ? <i>{hint}</i> : null}
          {trailing ? <span className="tr">{trailing}</span> : null}
        </div>
      ) : null}
      {hasChildren ? children : (empty ?? null)}
    </div>
  );
}

export interface MeterRowProps {
  /** Numbers are zero-padded to two digits; pass a string to opt out. */
  rank?: number | string;
  name: React.ReactNode;
  /** Share of the row's meter, 0–100. Clamped. */
  pct?: number;
  value?: React.ReactNode;
  /** Right-align the value as text rather than a figure. */
  valueText?: boolean;
  /**
   * Opens the in-volume inspector (gap A7). Deliberately NOT an href: row
   * activation is a depth change inside the projection, and the route out
   * lives on the Plane's `trailing` slot instead.
   */
  onClick?: () => void;
}

/** Rank / name / share meter / value. The dense ranked row. */
export function MeterRow({
  rank,
  name,
  pct = 0,
  value,
  valueText = false,
  onClick,
}: MeterRowProps) {
  return (
    <div
      className={valueText ? 'hp-rw hp-rw--text' : 'hp-rw'}
      onClick={onClick}
      role={onClick ? 'button' : undefined}
      tabIndex={onClick ? 0 : undefined}
      onKeyDown={
        onClick
          ? (e) => {
              if (e.key === 'Enter' || e.key === ' ') {
                e.preventDefault();
                onClick();
              }
            }
          : undefined
      }
    >
      <span className="rk">
        {typeof rank === 'number' ? String(rank).padStart(2, '0') : rank}
      </span>
      <span className="nm">{name}</span>
      <span className="mt">
        <i style={{ width: `${Math.max(0, Math.min(100, pct))}%` }} />
      </span>
      <span className="vv">{value}</span>
    </div>
  );
}

export interface LogRowProps {
  time: React.ReactNode;
  event: React.ReactNode;
  tone?: 'hot' | 'bad' | 'warn' | 'good';
  /** Overrides the tone-derived mark in the right column. */
  mark?: React.ReactNode;
}

/** Timestamp / event / mark. The event-log row. */
export function LogRow({ time, event, tone, mark }: LogRowProps) {
  const resolved =
    mark ?? (tone === 'bad' ? 'flagged' : tone === 'hot' ? 'marker' : '—');
  return (
    <div className="hp-lg">
      <span className="t">{time}</span>
      <span className={['ev', tone].filter(Boolean).join(' ')}>{event}</span>
      <span className="mx">{resolved}</span>
    </div>
  );
}
