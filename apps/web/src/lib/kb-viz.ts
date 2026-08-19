/**
 * Pure model for the contextual (visual) KB presentation. Given an
 * entity value and the peer-group quantiles for that metric, produce a
 * renderable row: a value string, a 0–100 fill position on the
 * min→max track, the median tick position, and a quantile band label.
 *
 * Degrades cleanly: when quantiles are missing (metric absent / n<5),
 * the row carries just the formatted value — no track, no band.
 */

import type { Quantiles } from './kb-stats';

export type Units = 'metric' | 'imperial';
export type BandTone = 'high' | 'mid' | 'low';

export interface BandLabel {
  text: string;
  tone: BandTone;
}

export interface StatRow {
  label: string;
  valueText: string;
  /** 0–100 position of the value on the min→max track; undefined when no stats. */
  fillPct?: number;
  /** 0–100 position of the class median tick; undefined when no stats. */
  medianPct?: number;
  band?: BandLabel;
}

const M_TO_FT = 3.28084;
const KG_TO_LB = 2.20462;

/** Units that convert to imperial; others (counts, aUEC, °/s) pass through. */
function convert(unit: string, value: number, units: Units): { value: number; unit: string } {
  if (units !== 'imperial') return { value, unit };
  if (unit === 'm/s') return { value: value * M_TO_FT, unit: 'ft/s' };
  if (unit === 'm') return { value: value * M_TO_FT, unit: 'ft' };
  if (unit === 'kg') return { value: value * KG_TO_LB, unit: 'lb' };
  return { value, unit };
}

function fmt(value: number, unit: string): string {
  const rounded = Math.abs(value) >= 100 ? Math.round(value) : Math.round(value * 100) / 100;
  const s = rounded.toLocaleString('en-US');
  return unit ? `${s} ${unit}` : s;
}

function clampPct(n: number): number {
  return Math.max(0, Math.min(100, n));
}

/** Position `x` on the [min,max] track as a 0–100 percentage. */
function trackPct(x: number, q: Quantiles): number {
  if (q.max === q.min) return 50;
  return clampPct(((x - q.min) / (q.max - q.min)) * 100);
}

/** Classify `x` into a quantile band with a human label + tone. */
export function bandLabel(x: number, q: Quantiles): BandLabel {
  if (x >= q.p90) return { text: 'top 10%', tone: 'high' };
  if (x <= q.p10) return { text: 'bottom 10%', tone: 'low' };
  if (x >= q.p75) return { text: 'top 25%', tone: 'high' };
  if (x <= q.p25) return { text: 'bottom 25%', tone: 'low' };
  return { text: '≈ median', tone: 'mid' };
}

export const TONE_COLOR: Record<BandTone, string> = {
  high: 'var(--success, #7Fd17F)',
  low: 'var(--danger, #e07a7a)',
  mid: 'var(--fg-muted)',
};

export function buildStatRow(
  label: string,
  unit: string,
  rawValue: number,
  q: Quantiles | undefined,
  units: Units,
): StatRow {
  const { value, unit: u } = convert(unit, rawValue, units);
  const valueText = fmt(value, u);
  if (!q || q.n < 5) {
    return { label, valueText };
  }
  return {
    label,
    valueText,
    fillPct: trackPct(rawValue, q),
    medianPct: trackPct(q.p50, q),
    band: bandLabel(rawValue, q),
  };
}
