/**
 * The sanctioned series palette — see the block comment in
 * `styles/additions.css` for why this exception exists and how narrow it is.
 *
 * ONLY for multi-series identity in a comparison chart. Never for state,
 * emphasis or a single series.
 *
 * The values are CSS variables rather than literals so the set recalibrates
 * with the beam. The anchor entity is deliberately not in here: it uses
 * `--hot`, so the thing you came to read outshines everything it is measured
 * against.
 */
export const SERIES_SLOTS = 6;

/** `--hot` for the anchor; a palette slot for each comparator. */
export function seriesColor(index: number, isAnchor = false): string {
  if (isAnchor) return 'var(--hot)';
  return `var(--hp-series-${(index % SERIES_SLOTS) + 1})`;
}

/**
 * Dash pattern for a series past the sixth, where the ramp repeats.
 *
 * Returning `undefined` for the first six is the point: a solid stroke is the
 * default and dashes only appear when colour alone has stopped being enough to
 * tell two series apart.
 */
export function seriesDash(index: number): string | undefined {
  const cycle = Math.floor(index / SERIES_SLOTS);
  if (cycle === 0) return undefined;
  return cycle === 1 ? '5 3' : '2 3';
}
