'use client';

import React from 'react';

/**
 * A leader-lined readout pinned in the volume. Callouts are the overview's
 * verbosity: six of them ring the core, each with a label, figure and a line
 * of supporting arithmetic. While editing the layout each carries a remove
 * control.
 *
 * CAPACITY (measured, not assumed — `preview-stress.html`): six is the drawn
 * maximum, three a side. A run at nine put five of them over the ring. The
 * whole field is hidden below 1180px, where the ring and pane leave no clear
 * air either side — so NEVER put a metric only in a callout. Every figure
 * here must also be reachable from a lens pane.
 */
export interface CalloutPosition {
  left?: number;
  right?: number;
  top?: number;
}

export interface CalloutProps {
  label: React.ReactNode;
  value: React.ReactNode;
  unit?: React.ReactNode;
  /** The supporting arithmetic line beneath the figure. */
  sub?: React.ReactNode;
  side?: 'l' | 'r';
  /** Tone applies to the FIGURE only; the label stays dim. */
  tone?: 'warn' | 'good' | 'bad';
  /** Absolute position in the stage. Use `slotFor()` rather than hand-placing. */
  at?: CalloutPosition;
  /** Passed only while the layout editor is open — this is what keeps the × out
   *  of the reading view. */
  onRemove?: () => void;
  children?: React.ReactNode;
}

export function Callout({
  label,
  value,
  unit,
  sub,
  side = 'l',
  tone,
  at,
  onRemove,
  children,
}: CalloutProps) {
  const cls = ['hp-co', tone].filter(Boolean).join(' ');
  const style: React.CSSProperties = {
    ...(at || {}),
    ...(side === 'r' ? { textAlign: 'left' as const } : null),
  };
  return (
    <div className={cls} data-side={side} style={style}>
      <span className="ln" />
      {onRemove ? (
        <button
          type="button"
          className="rm"
          aria-label={`Remove ${typeof label === 'string' ? label : 'callout'}`}
          onClick={onRemove}
        >
          ×
        </button>
      ) : null}
      <div className="lb">{label}</div>
      <div className="vl">
        {value}
        {unit ? <small>{unit}</small> : null}
      </div>
      {sub ? <div className="sub">{sub}</div> : null}
      {children}
    </div>
  );
}

/**
 * The fixed callout slots (gap B5).
 *
 * Upstream, every callout id was pinned to a literal `{left:76, top:212}` in a
 * hand-authored map — fine for a kit with a fixed cast, wrong for a product
 * where the reader chooses which of ~8 single-figure metrics to project. So
 * positions become SLOTS and the reader's enabled callouts fill them in layout
 * order; removing one frees its slot and the rest shuffle up, which is exactly
 * what the component's own notes describe ("slots free up in order").
 *
 * Coordinates are lifted from the kit's primary six so the composition is the
 * one that was actually drawn and stress-tested. Left column fills first.
 *
 * THE SEVENTH EXISTS BECAUSE THE CATALOGUE OFFERS SEVEN. With six, a reader
 * who enabled every callout was told "+1 undrawn · reorder" permanently — a
 * true statement about capacity, and an unactionable one: reordering only
 * changes WHICH six appear, so there was no arrangement that satisfied it.
 * Being told part of your projection cannot be shown, with no remedy, reads
 * as a fault however the label is worded (this was the second attempt at
 * wording it). The extra slot continues the left column's 154px rhythm and is
 * only ever filled when all seven are on. It is kept clear of the two things
 * that sit low on the stage — the hint is bottom-RIGHT and the lens rail is
 * bottom-CENTRE, while this is on the left — so the deepest slot has no
 * horizontal overlap with either.
 */
export const CALLOUT_SLOTS: readonly { side: 'l' | 'r'; at: CalloutPosition }[] =
  [
    { side: 'l', at: { left: 76, top: 212 } },
    { side: 'l', at: { left: 54, top: 366 } },
    { side: 'l', at: { left: 96, top: 520 } },
    { side: 'r', at: { right: 74, top: 204 } },
    { side: 'r', at: { right: 92, top: 358 } },
    { side: 'r', at: { right: 66, top: 512 } },
    { side: 'l', at: { left: 68, top: 674 } },
  ];

/** Slot for the Nth projected callout, or `null` past capacity (CalloutField
 *  reports the overflow rather than drawing it). */
export function slotFor(
  index: number,
): { side: 'l' | 'r'; at: CalloutPosition } | null {
  return CALLOUT_SLOTS[index] ?? null;
}

/**
 * Wrapper for the callout set. Caps at `max` — the number of slots that
 * exist, because past that they overlap the ring and each other — and reports
 * the remainder rather than silently dropping it. The default is derived from
 * `CALLOUT_SLOTS` rather than written out: a literal that had to agree with
 * the array is exactly how the two drifted apart. Extra metrics stay
 * reachable in the pane and the layout editor.
 *
 * The whole field is hidden below 1180px, where the ring and pane leave no
 * clear air either side.
 */
export function CalloutField({
  children,
  max = CALLOUT_SLOTS.length,
  onOverflowClick,
}: {
  children?: React.ReactNode;
  max?: number;
  onOverflowClick?: () => void;
}) {
  const all = React.Children.toArray(children);
  const shown = max > 0 ? all.slice(0, max) : all;
  const hidden = all.length - shown.length;
  return (
    <div className="hp-cos">
      {shown}
      {hidden > 0 ? (
        /**
         * "+N more in layout" NAMED THE WRONG CAUSE.
         *
         * It reads as "there are N you have not added", so a reader with
         * everything already enabled is told to go and find them — and the
         * button opens the layout editor, where every one of them is present
         * and ticked. A dead end that blames the reader for the field's own
         * limit.
         *
         * The truth is capacity: six slots, three a side, past which callouts
         * overlap the ring and each other. The remainder is not missing, it is
         * undrawn — and the ORDER is what decides which six, which is the one
         * thing the editor can actually change. So the label says what is
         * happening and the title says what to do about it.
         */
        <button
          type="button"
          className="hp-cos-more"
          onClick={onOverflowClick}
          title={`The field draws ${max}. ${hidden} more ${hidden === 1 ? 'is' : 'are'} enabled but undrawn — reorder your layout to choose which ${max} appear.`}
        >
          +{hidden} undrawn · reorder
        </button>
      ) : null}
    </div>
  );
}
