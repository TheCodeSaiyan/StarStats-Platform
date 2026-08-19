/**
 * Period-over-period trend: this window against the one before it.
 *
 * This is a different comparison from the `lifetime` twin. Lifetime
 * answers "how much of my career is this"; trend answers "is this
 * getting better or worse". Trend is the one that belongs at the front
 * of a dashboard tile, because direction is what a glance is for.
 *
 * One helper so all six widgets phrase it identically — the same reason
 * `rangeHasLifetimeBaseline` lives in `range.ts` rather than being
 * re-decided per widget.
 */
import type { RangeId } from './range';
import { RANGES } from './range';

export type TrendDirection = 'up' | 'down' | 'flat' | 'first';

export interface Trend {
  direction: TrendDirection;
  /** Signed absolute change. 0 when flat. */
  delta: number;
  /**
   * Signed percent change, or null when a percentage would mislead.
   *
   * Null in two cases: the previous window was 0 (the change is not a
   * percentage of anything — that is `first`, not "+∞%"), and when the
   * previous window is below {@link PCT_FLOOR}, where a percentage
   * exaggerates. Going from 1 to 2 is "+1", not "+100%" — the latter
   * reads as a trend when it is a rounding-scale wobble.
   */
  pct: number | null;
}

/**
 * Below this, a percentage is noise rather than signal, so only the
 * absolute delta is reported. Chosen so a single event cannot produce a
 * double-digit percentage swing.
 */
export const PCT_FLOOR = 10;

/**
 * Compare a window against its predecessor.
 *
 * `previous` must be the REAL previous-window figure. Passing 0 for
 * "we don't know" is the one thing that breaks this: it is
 * indistinguishable from a genuine zero and renders as `first`,
 * claiming the player did nothing last period when in truth we never
 * asked. The server distinguishes the two — an absent `previous` twin
 * means "no comparison to draw" — so callers must skip rendering
 * entirely rather than substituting a number.
 */
export function computeTrend(current: number, previous: number): Trend {
  if (previous === 0) {
    // Both zero is flat, not "first" — nothing happened either window,
    // which is a real (if dull) comparison rather than a debut.
    if (current === 0) return { direction: 'flat', delta: 0, pct: null };
    return { direction: 'first', delta: current, pct: null };
  }
  const delta = current - previous;
  if (delta === 0) return { direction: 'flat', delta: 0, pct: null };
  const pct =
    previous >= PCT_FLOOR ? Math.round((delta / previous) * 100) : null;
  return { direction: delta > 0 ? 'up' : 'down', delta, pct };
}

/** Short label for the window a trend is measured against ("7d"). */
export function previousWindowLabel(id: RangeId): string {
  return RANGES.find((r) => r.id === id)?.label ?? id;
}

const ARROW: Record<TrendDirection, string> = {
  up: '▲',
  down: '▼',
  flat: '=',
  first: '▲',
};

/**
 * Render a trend as tile copy.
 *
 * The arrow is decoration, never the message: the text alone carries
 * direction ("+2", "−2", "no change"), so the meaning survives a
 * screen reader, a monochrome render, and a glyph that fails to load.
 *
 * `unit` is appended to the absolute delta when given ("aUEC"), and
 * `fmt` formats the magnitude (pass the widget's `fmtNum`).
 */
export function formatTrend(
  t: Trend,
  rangeLabel: string,
  fmt: (n: number) => string,
  unit?: string,
): string {
  const suffix = unit ? ` ${unit}` : '';
  const vs = `vs prev ${rangeLabel}`;

  if (t.direction === 'flat') return `${ARROW.flat} no change ${vs}`;
  if (t.direction === 'first') {
    // Not "+100%" and not "+N vs 0": the previous window was empty, so
    // there is no ratio and no baseline to subtract from. Say that.
    return `${ARROW.first} ${fmt(t.delta)}${suffix} — none in the prev ${rangeLabel}`;
  }
  const sign = t.delta > 0 ? '+' : '−';
  const magnitude = `${sign}${fmt(Math.abs(t.delta))}${suffix}`;
  const pct = t.pct === null ? '' : ` (${sign}${Math.abs(t.pct)}%)`;
  return `${ARROW[t.direction]} ${magnitude}${pct} ${vs}`;
}
