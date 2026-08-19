/**
 * Provenance affordance for an AGGREGATE metric — UX Rule 4.
 *
 * `InferredBadge` answers "is this row inferred?" for a single event.
 * Aggregates need a different question answered: "how much of this
 * number is inferred?" Summing rows away is exactly what loses the
 * per-row provenance flag, so an aggregate hides its own uncertainty
 * unless the split travels with the total.
 *
 * The case that motivated this: CIG removed the Actor Death log lines,
 * so many deaths are reconstructed from a `Corpse` line rather than read
 * directly. A widget showing "12 deaths" gives a user no way to know
 * that some of those were reconstructed.
 *
 * Deliberately a sibling of `InferredBadge` rather than a change to it —
 * that component's two call sites are correct and must not shift.
 */

import React from 'react';

interface Props {
  /** Total the surface is displaying. */
  total: number;
  /** How many of `total` were inferred rather than observed. */
  inferred: number;
  /** How the inference was made, in plain words. Shown on hover and to
   *  screen readers — a provenance marker that cannot say WHY is just
   *  an unexplained asterisk. */
  note: string;
  children: React.ReactNode;
}

/**
 * Wraps a value, marking it only when part of it is inferred.
 *
 * Renders the children unchanged when `inferred <= 0`. That is the
 * important half: badging every number teaches people to ignore the
 * badge, and the signal is only worth having while it is rare.
 */
export function Provenance({ total, inferred, note, children }: Props) {
  // Fully observed — no marker at all, not a "0% inferred" marker.
  if (!Number.isFinite(inferred) || inferred <= 0) return <>{children}</>;

  // Clamp to the total: a split larger than its total means a bug
  // upstream, and rendering "15 of 12" would broadcast that bug to the
  // user as a finding. Clamping shows "12 of 12" instead — still wrong
  // upstream, but not self-evidently absurd on screen.
  const part = Math.min(inferred, total);
  const label = `${part} of ${total} inferred, not observed — ${note}`;

  return (
    <span
      role="note"
      aria-label={label}
      title={label}
      style={{
        textDecorationLine: 'underline',
        textDecorationStyle: 'dotted',
        textUnderlineOffset: 3,
        cursor: 'help',
      }}
    >
      {children}
      <span aria-hidden="true" style={{ marginLeft: 4, opacity: 0.7, fontSize: '0.85em' }}>
        ⓘ
      </span>
    </span>
  );
}
