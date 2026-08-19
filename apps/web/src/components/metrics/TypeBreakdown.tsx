/**
 * TypeBreakdown — donut + ranked bar list of event types.
 *
 * Replaces the metrics page's manual `<div>` bar chart in the Types
 * tab. Uses recharts `<PieChart>` for the donut; ranked list is plain
 * SVG/CSS bars so it stays theme-reactive.
 *
 * Top N types render as distinct accent shades; the rest collapse
 * into a single "Other" wedge so the donut stays readable.
 */

'use client';

import { useChartTheme, type ChartTheme } from '@/lib/recharts-theme';
import { formatEventType } from '@/lib/event-types';
import { ChartCard } from './ChartCard';
import { Cell, Pie, PieChart, ResponsiveContainer, Tooltip } from 'recharts';

export interface TypeBreakdownRow {
  event_type: string;
  count: number;
}

export interface TypeBreakdownProps {
  types: TypeBreakdownRow[];
  caption?: string;
  topN?: number;
}

function buildRamp(theme: ChartTheme): string[] {
  return [theme.accent, theme.grid[3], theme.grid[2], theme.grid[1], theme.fgDim, theme.fgDim];
}

export function TypeBreakdown(props: TypeBreakdownProps) {
  const { types, caption, topN = 5 } = props;
  const theme = useChartTheme();
  const total = types.reduce((s, t) => s + t.count, 0);
  const mode = types.length === 0 || total === 0 ? 'empty' : 'data';

  const sorted = [...types].sort((a, b) => b.count - a.count);
  const top = sorted.slice(0, topN);
  const rest = sorted.slice(topN);
  const restCount = rest.reduce((s, t) => s + t.count, 0);
  // Decorate each slice with the humanized label + glyph once so
  // both the chart nameKey and the visible list draw from one
  // source. "Other" is a synthetic aggregator (not a real
  // event_type), so it gets a static presentation rather than a
  // mapper lookup that would title-case it.
  const decorate = (row: TypeBreakdownRow, isOther = false) => {
    if (isOther) {
      return { ...row, label: 'Other', glyph: '⋯' };
    }
    const meta = formatEventType(row.event_type);
    return { ...row, label: meta.label, glyph: meta.glyph };
  };
  const slices =
    restCount > 0
      ? [
          ...top.map((t) => decorate(t)),
          decorate({ event_type: 'Other', count: restCount }, true),
        ]
      : top.map((t) => decorate(t));
  const ramp = buildRamp(theme);

  return (
    <ChartCard
      title="Event types"
      caption={caption ?? `${total.toLocaleString()} events`}
      flagKey="metrics.type_breakdown"
      telemetryKey="type_breakdown"
      mode={mode}
      empty={<span style={{ color: 'var(--fg-muted)' }}>No events to break down.</span>}
      error={<span>Couldn’t load event-type breakdown.</span>}
      height={260}
      srTable={
        <table>
          <caption>Event types ranked by count</caption>
          <thead>
            <tr>
              <th>Type</th>
              <th>Count</th>
              <th>Share</th>
            </tr>
          </thead>
          <tbody>
            {sorted.map((t) => (
              <tr key={t.event_type}>
                <td>{formatEventType(t.event_type).label}</td>
                <td>{t.count}</td>
                <td>{total > 0 ? `${((t.count / total) * 100).toFixed(1)}%` : '—'}</td>
              </tr>
            ))}
          </tbody>
        </table>
      }
    >
      <div className="type-breakdown" style={{ display: 'flex', gap: 'var(--s5)', height: '100%' }}>
        <div style={{ flex: '0 0 200px', height: '100%' }}>
          <ResponsiveContainer width="100%" height="100%">
            <PieChart>
              <Pie
                data={slices}
                dataKey="count"
                nameKey="label"
                innerRadius="55%"
                outerRadius="92%"
                stroke="none"
              >
                {slices.map((s, i) => (
                  <Cell key={s.event_type} fill={ramp[Math.min(i, ramp.length - 1)]} />
                ))}
              </Pie>
              <Tooltip
                contentStyle={{
                  background: 'var(--surface-2)',
                  border: `1px solid ${theme.border}`,
                  borderRadius: 8,
                  color: 'var(--fg)',
                  fontSize: 'var(--fs-sm)',
                }}
                formatter={(v, name) => {
                  const num = typeof v === 'number' ? v : Number(v ?? 0);
                  const pct = total > 0 ? ((num / total) * 100).toFixed(1) : '0.0';
                  return [`${num.toLocaleString()} (${pct}%)`, name as string];
                }}
              />
            </PieChart>
          </ResponsiveContainer>
        </div>
        <ol
          className="type-breakdown__list"
          style={{ flex: 1, margin: 0, padding: 0, listStyle: 'none' }}
        >
          {slices.map((s, i) => {
            const pct = total > 0 ? (s.count / total) * 100 : 0;
            const color = ramp[Math.min(i, ramp.length - 1)];
            return (
              <li
                key={s.event_type}
                style={{
                  display: 'grid',
                  gridTemplateColumns: 'minmax(0, 1fr) 88px',
                  alignItems: 'center',
                  gap: 'var(--s2)',
                  marginBottom: 'var(--s2)',
                  fontSize: 'var(--fs-sm)',
                  color: 'var(--fg)',
                }}
              >
                <div>
                  <div
                    style={{
                      display: 'flex',
                      alignItems: 'center',
                      gap: 'var(--s2)',
                      marginBottom: 4,
                    }}
                  >
                    <span
                      style={{
                        width: 8,
                        height: 8,
                        borderRadius: '50%',
                        background: color,
                        flex: '0 0 8px',
                      }}
                    />
                    <span
                      style={{
                        overflow: 'hidden',
                        textOverflow: 'ellipsis',
                        display: 'inline-flex',
                        gap: 6,
                        alignItems: 'baseline',
                      }}
                      title={s.event_type}
                    >
                      <span aria-hidden="true">{s.glyph}</span>
                      <span>{s.label}</span>
                    </span>
                  </div>
                  <div
                    style={{
                      width: `${Math.max(2, pct)}%`,
                      height: 4,
                      background: color,
                      borderRadius: 2,
                    }}
                  />
                </div>
                <div style={{ textAlign: 'right', color: 'var(--fg-muted)' }}>
                  {s.count.toLocaleString()} · {pct.toFixed(1)}%
                </div>
              </li>
            );
          })}
        </ol>
      </div>
    </ChartCard>
  );
}
