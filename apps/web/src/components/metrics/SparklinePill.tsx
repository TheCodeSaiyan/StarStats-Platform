/**
 * SparklinePill — a stat pill backed by a tiny inline-SVG sparkline.
 *
 * Renders the headline number (e.g. "23 kills") above a trend line.
 * Skips the full `MetricCard` shell because pills are small and
 * dense — they live in a strip of 3-5 pills, not as standalone cards.
 */

'use client';

import { isFlagEnabled } from '@/lib/feature-flags';
import { Sparkline } from './Sparkline';

export interface SparklinePillProps {
  value: string;
  label: string;
  series: number[];
  caption?: string;
}

export function SparklinePill(props: SparklinePillProps) {
  const { value, label, series, caption } = props;
  if (!isFlagEnabled('metrics.sparkline_pills')) return null;

  const hasData = series.length > 0 && series.some((v) => v > 0);

  return (
    <div className="ss-stat sparkline-pill" role="group" aria-label={`${value} ${label}`}>
      <div className="sparkline-pill__head">
        <div className="sparkline-pill__value">{value}</div>
        <div className="sparkline-pill__label">{label}</div>
      </div>
      {hasData ? (
        // Delegate the SVG + path math to the shared Sparkline primitive
        // so there is a single source of truth for how a trend is drawn.
        <Sparkline
          className="sparkline-pill__svg"
          series={series}
          height={28}
          label={`${label} trend over ${series.length} points`}
        />
      ) : (
        <div className="sparkline-pill__svg sparkline-pill__svg--empty" aria-hidden="true" />
      )}
      {caption ? <div className="sparkline-pill__caption">{caption}</div> : null}
    </div>
  );
}
