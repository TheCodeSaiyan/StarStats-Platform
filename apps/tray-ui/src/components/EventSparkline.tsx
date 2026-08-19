/**
 * EventSparkline — inline-SVG sparkline of events/hour for the tray.
 *
 * Takes a `TimelineEntry[]` (newest-first, from `getSessionTimeline()`),
 * buckets into hourly counts over the last 48 hours, and draws a
 * 220×36 SVG line against the `--accent` token so theme swaps repaint
 * automatically. No chart library — the tray binary already carries
 * enough weight.
 */

import type { TimelineEntry } from '../api';

const W = 220;
const H = 36;
const BUCKETS = 48; // hours

function bucketize(entries: TimelineEntry[]): number[] {
  const now = Date.now();
  const hourMs = 60 * 60 * 1000;
  const buckets = new Array<number>(BUCKETS).fill(0);
  for (const e of entries) {
    const ts = Date.parse(e.timestamp);
    if (!Number.isFinite(ts)) continue;
    const age = now - ts;
    if (age < 0 || age >= BUCKETS * hourMs) continue;
    const idx = BUCKETS - 1 - Math.floor(age / hourMs);
    if (idx >= 0 && idx < BUCKETS) buckets[idx] += 1;
  }
  return buckets;
}

function buildPath(series: number[]): string {
  if (series.length === 0) return '';
  const max = Math.max(...series, 1);
  const stepX = W / Math.max(series.length - 1, 1);
  return series
    .map((v, i) => {
      const x = i * stepX;
      const y = H - (v / max) * (H - 2) - 1;
      return `${i === 0 ? 'M' : 'L'}${x.toFixed(1)},${y.toFixed(1)}`;
    })
    .join(' ');
}

function buildArea(series: number[]): string {
  const line = buildPath(series);
  if (line.length === 0) return '';
  return `${line} L${W.toFixed(1)},${H.toFixed(1)} L0,${H.toFixed(1)} Z`;
}

export interface EventSparklineProps {
  entries: TimelineEntry[];
}

export function EventSparkline({ entries }: EventSparklineProps) {
  const series = bucketize(entries);
  const total = series.reduce((s, v) => s + v, 0);
  const hasData = total > 0;
  const peak = Math.max(...series);

  return (
    <div
      style={{ display: 'flex', flexDirection: 'column', gap: 4 }}
      role="group"
      aria-label={`${total} events in the last 48 hours`}
    >
      <div
        style={{
          display: 'flex',
          alignItems: 'baseline',
          gap: 8,
          fontSize: 12,
        }}
      >
        <span
          style={{
            fontFamily: 'var(--font-mono)',
            fontSize: 16,
            fontWeight: 600,
            color: 'var(--fg)',
          }}
        >
          {total.toLocaleString()}
        </span>
        <span
          style={{
            color: 'var(--fg-muted)',
            textTransform: 'uppercase',
            letterSpacing: '0.06em',
          }}
        >
          events · 48h
        </span>
        {hasData ? (
          <span
            style={{ marginLeft: 'auto', color: 'var(--fg-dim)', fontSize: 11 }}
          >
            peak {peak}/h
          </span>
        ) : null}
      </div>
      {hasData ? (
        <svg
          width={W}
          height={H}
          viewBox={`0 0 ${W} ${H}`}
          style={{ display: 'block', width: '100%', maxWidth: W }}
          role="img"
          aria-label={`Hourly event count over the last ${BUCKETS} hours`}
        >
          <path d={buildArea(series)} fill="var(--accent-soft)" stroke="none" />
          <path
            d={buildPath(series)}
            fill="none"
            stroke="var(--accent)"
            strokeWidth={1.5}
            strokeLinecap="round"
            strokeLinejoin="round"
          />
        </svg>
      ) : (
        <div
          style={{ height: H, background: 'var(--surface-2)', borderRadius: 4 }}
          aria-hidden="true"
        />
      )}
    </div>
  );
}
