import React from 'react';
import { SubStats } from 'holo';
import type { StatRow } from '@/lib/kb-viz';

/**
 * The pinned headline metrics at the top of the visual view.
 *
 * REDRAWN. This was four rounded, filled cards with a hardcoded
 * `#16131d` background and a 10px radius — the system has no filled card and
 * no radius anywhere. `SubStats` is its answer for a row of figures: the value
 * is the figure and glows, the label is dim and tracked.
 *
 * The quantile band ("faster than 82% of light fighters") rides along as the
 * sub-line rather than as coloured text. Tone is carried by the system's own
 * `tone` rather than `TONE_COLOR`, because the flat palette does not follow the
 * beam when a reader recalibrates — a "good" green stayed green on Pyro.
 */
const BAND_TONE: Record<string, 'good' | 'warn' | 'bad' | undefined> = {
  good: 'good',
  warn: 'warn',
  bad: 'bad',
};

export function HeadlineCallouts({ rows }: { rows: StatRow[] }) {
  if (rows.length === 0) return null;
  return (
    <SubStats
      items={rows.slice(0, 4).map((r) => ({
        k: r.label,
        v: r.valueText,
        sub: r.band?.text,
        tone: r.band ? BAND_TONE[r.band.tone] : undefined,
      }))}
    />
  );
}
