import React from 'react';
import { NoSignal } from '@/components/hud/NoSignal';
import { fmtNum } from './format';

/**
 * "Nothing in THIS window, but you do have some" — the state between a
 * populated tile and a genuinely empty account.
 *
 * Why this exists: `routes`, `spend`, `docking`, `objectives` and
 * `fleet` were lifetime-only until #309 (2026-07-23) range-scoped them.
 * From then on, a handle whose activity predates the selected range got
 * an empty list, the widget returned `null`, and the tile rendered the
 * same blank box a brand-new account gets. That is indistinguishable
 * from a broken feature — and was reported as exactly that ("routes are
 * not actually populating"), four days after routes shipped working.
 *
 * The lifetime twin knows the difference, so the tile says it: both
 * figures and the action that fixes it. Only use this when there IS a
 * lifetime figure to name — a genuinely empty account must still render
 * nothing, because telling someone to widen a range that holds nothing
 * wider is worse than silence.
 */
export interface EmptyWindowProps {
  /** Range selector label for the current window, e.g. "7d". */
  rangeLabel: string;
  /** Lifetime total. Only render this component when it is > 0. */
  lifetimeCount: number;
  /** Plural noun for what is missing, e.g. "quantum routes". */
  noun: string;
}

export function EmptyWindow({ rangeLabel, lifetimeCount, noun }: EmptyWindowProps) {
  return (
    <NoSignal
      compact
      title="Nothing in this window"
      hint={`No ${noun} in the last ${rangeLabel} — ${fmtNum(
        lifetimeCount,
      )} all time. Widen the range to see them.`}
    />
  );
}
