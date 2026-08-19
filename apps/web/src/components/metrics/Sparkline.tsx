/**
 * Sparkline — a tiny, axis-less inline line chart.
 *
 * Pure presentational and hook-free, so it renders inside SERVER
 * component bodies (e.g. profile widget renders) as well as client
 * components. Theme-aware via design tokens: the line uses
 * `var(--accent)` and the optional area fill `var(--accent-soft)`, both
 * of which flip with the active theme.
 *
 * It carries NO layout opinion beyond a small fixed height — width is
 * flexible so it can sit inline beside a numeric readout. Empty and
 * single-point series degrade safely (nothing / a flat baseline) rather
 * than crashing on the min/max math.
 */

import React from 'react';

export interface SparklineProps {
  /** Numeric series, oldest-first (left -> right). */
  series: number[];
  /**
   * Short screen-reader label describing the trend, e.g.
   * "playtime, last 5 sessions". The sparkline conveys a trend the
   * adjacent number does NOT, so it is exposed as an image with a
   * label rather than hidden.
   */
  label: string;
  /** Drawing width in px (viewBox units). Flexible; default 96. */
  width?: number;
  /** Drawing height in px (viewBox units). Tiny; default 18. */
  height?: number;
  /** Faint filled area under the line. Default true. */
  area?: boolean;
  className?: string;
}

/** Build the polyline `d` for a series across a `w`×`h` box. Assumes
 *  `series.length >= 2`; single/empty series are normalised by the
 *  caller. Min/max are clamped so a flat series draws a level line
 *  instead of dividing by zero. */
function buildPath(series: number[], w: number, h: number): string {
  const max = Math.max(...series);
  const min = Math.min(...series);
  const range = Math.max(max - min, 1);
  const stepX = w / Math.max(series.length - 1, 1);
  // Inset the line by half its stroke so it isn't clipped at the edges.
  const pad = 1;
  const usableH = h - pad * 2;
  return series
    .map((v, i) => {
      const x = i * stepX;
      const y = pad + (usableH - ((v - min) / range) * usableH);
      return `${i === 0 ? 'M' : 'L'}${x.toFixed(1)},${y.toFixed(1)}`;
    })
    .join(' ');
}

export function Sparkline({
  series,
  label,
  width = 96,
  height = 18,
  area = true,
  className,
}: SparklineProps) {
  // Nothing to plot — render nothing so the caller's layout collapses.
  if (series.length === 0) return null;

  // A single point can't form a line; duplicate it into a flat pair so
  // the sparkline reads as a level baseline rather than an empty box.
  const pts = series.length === 1 ? [series[0], series[0]] : series;

  const line = buildPath(pts, width, height);
  const areaPath = area
    ? `${line} L${width.toFixed(1)},${height.toFixed(1)} L0,${height.toFixed(1)} Z`
    : null;

  return (
    <svg
      className={className}
      width={width}
      height={height}
      viewBox={`0 0 ${width} ${height}`}
      preserveAspectRatio="none"
      role="img"
      aria-label={label}
      style={{ display: 'block', overflow: 'visible' }}
    >
      {areaPath ? (
        <path d={areaPath} fill="var(--accent-soft)" stroke="none" />
      ) : null}
      <path
        d={line}
        fill="none"
        stroke="var(--accent)"
        strokeWidth={1.5}
        strokeLinecap="round"
        strokeLinejoin="round"
        vectorEffect="non-scaling-stroke"
      />
    </svg>
  );
}
