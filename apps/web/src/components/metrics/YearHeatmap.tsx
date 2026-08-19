/**
 * GitHub-style year heatmap — 53 columns (weeks) × 7 rows (Sun-Sat).
 *
 * Successor to `DayHeatmap` (30-day grid). Lives inside a `MetricCard`
 * shell so it inherits the empty / error / sr-table / feature-flag
 * contract from §2 of the impl plan.
 *
 * Renders as inline SVG against the `--grid-*` token ladder so theme
 * swaps repaint without a re-mount.
 */

'use client';

import { MetricCard } from './MetricCard';

export interface YearHeatmapBucket {
  count: number;
  /** YYYY-MM-DD in UTC. */
  date: string;
}

export interface YearHeatmapData {
  buckets: YearHeatmapBucket[];
  days: number;
}

const CELL = 11;
const GAP = 2;
const WEEKS = 53;
const DAYS_IN_WEEK = 7;

function levelFor(count: number, max: number): 0 | 1 | 2 | 3 | 4 {
  if (count === 0 || max === 0) return 0;
  const ratio = count / max;
  if (ratio < 0.25) return 1;
  if (ratio < 0.5) return 2;
  if (ratio < 0.75) return 3;
  return 4;
}

function levelColor(level: 0 | 1 | 2 | 3 | 4): string {
  return `var(--grid-${level === 0 ? 'empty' : level})`;
}

export interface YearHeatmapProps {
  timeline: YearHeatmapData;
  title?: string;
  caption?: string;
}

export function YearHeatmap({ timeline, title = 'Year of activity', caption }: YearHeatmapProps) {
  const buckets = timeline.buckets;
  const max = buckets.reduce((m, b) => (b.count > m ? b.count : m), 0);
  const total = buckets.reduce((s, b) => s + b.count, 0);
  const mode = buckets.length === 0 ? 'empty' : 'data';

  const byDate = new Map<string, YearHeatmapBucket>();
  for (const b of buckets) byDate.set(b.date, b);

  const today = new Date();
  today.setHours(12, 0, 0, 0);

  // GitHub-style layout: the rightmost column ends on today's row;
  // the leftmost column is the Sunday 52 weeks back from this
  // week's Sunday. Walk Sunday→Saturday for each of the 53 weeks
  // so x = week index (0..52) and y = day-of-week (0..6) stay
  // consistent with the calendar position of `day`.
  const todayDow = today.getDay();
  const leftmostSunday = new Date(today);
  leftmostSunday.setDate(today.getDate() - todayDow - (WEEKS - 1) * DAYS_IN_WEEK);

  const cells: Array<{
    x: number;
    y: number;
    date: string;
    count: number;
    level: 0 | 1 | 2 | 3 | 4;
  }> = [];

  for (let week = 0; week < WEEKS; week++) {
    for (let dow = 0; dow < DAYS_IN_WEEK; dow++) {
      const day = new Date(leftmostSunday);
      day.setDate(leftmostSunday.getDate() + week * DAYS_IN_WEEK + dow);
      // Skip future days — the rightmost column ends at today, but the
      // grid is 7-row so future cells in today's column are absent
      // rather than rendered as empty (mirrors GitHub's behaviour).
      if (day > today) continue;
      const iso = `${day.getFullYear()}-${String(day.getMonth() + 1).padStart(2, '0')}-${String(
        day.getDate(),
      ).padStart(2, '0')}`;
      const bucket = byDate.get(iso);
      const count = bucket?.count ?? 0;
      cells.push({ x: week, y: dow, date: iso, count, level: levelFor(count, max) });
    }
  }

  const width = WEEKS * (CELL + GAP);
  const height = DAYS_IN_WEEK * (CELL + GAP);

  return (
    <MetricCard
      title={title}
      caption={caption ?? `${total.toLocaleString()} events · ${timeline.days} days`}
      flagKey="metrics.year_heatmap"
      telemetryKey="year_heatmap"
      mode={mode}
      empty={<span style={{ color: 'var(--fg-muted)' }}>No activity in this window.</span>}
      error={<span>Couldn’t load the activity heatmap.</span>}
      srTable={
        <table>
          <caption>Activity per day, most recent {timeline.days} days</caption>
          <thead>
            <tr>
              <th>Date</th>
              <th>Events</th>
            </tr>
          </thead>
          <tbody>
            {buckets.map((b) => (
              <tr key={b.date}>
                <td>{b.date}</td>
                <td>{b.count}</td>
              </tr>
            ))}
          </tbody>
        </table>
      }
    >
      <div className="year-heatmap" style={{ overflowX: 'auto' }}>
        <svg
          width={width}
          height={height}
          viewBox={`0 0 ${width} ${height}`}
          role="img"
          aria-label={`${total} events across the last ${timeline.days} days`}
        >
          {cells.map((c) => (
            <rect
              key={c.date}
              x={c.x * (CELL + GAP)}
              y={c.y * (CELL + GAP)}
              width={CELL}
              height={CELL}
              rx={2}
              ry={2}
              fill={levelColor(c.level)}
            >
              <title>{`${c.date} · ${c.count} event${c.count === 1 ? '' : 's'}`}</title>
            </rect>
          ))}
        </svg>
      </div>
    </MetricCard>
  );
}
