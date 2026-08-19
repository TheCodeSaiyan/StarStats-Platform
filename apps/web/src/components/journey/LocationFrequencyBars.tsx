/**
 * Horizontal dwell-time bars. Answers "where do I spend most of my
 * time?" over the rolling window the caller asks for (default 7 days
 * via `getLocationBreakdown`).
 *
 * Replaces the inline `DwellChart` from /journey. Server component;
 * caller fetches the `BreakdownResponse` and passes its entries in.
 */

import type { BreakdownResponse } from '@/lib/api';
import { formatDwell } from './trail-utils';

interface Props {
  entries: BreakdownResponse['entries'];
  /** How many rows to render — long tails add noise. Default 8. */
  topN?: number;
}

export function LocationFrequencyBars({ entries, topN = 8 }: Props) {
  if (!entries || entries.length === 0) {
    return (
      <p style={{ margin: 0, color: 'var(--fg-dim)', fontSize: 13 }}>
        Not enough data to chart yet.
      </p>
    );
  }

  const rows = [...entries]
    .sort((a, b) => b.dwell_seconds - a.dwell_seconds)
    .slice(0, topN);
  const max = Math.max(...rows.map((r) => r.dwell_seconds), 1);
  const totalSec = rows.reduce((sum, r) => sum + r.dwell_seconds, 0);

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 10 }}>
      <ol
        style={{
          listStyle: 'none',
          margin: 0,
          padding: 0,
          display: 'flex',
          flexDirection: 'column',
          gap: 10,
        }}
      >
        {rows.map((e, i) => {
          const label = e.city ?? e.planet ?? e.system ?? 'Unknown';
          const pct = (e.dwell_seconds / max) * 100;
          return (
            <li key={`${label}-${i}`}>
              <div
                style={{
                  display: 'flex',
                  justifyContent: 'space-between',
                  alignItems: 'baseline',
                  fontSize: 12,
                  marginBottom: 4,
                  gap: 8,
                }}
              >
                <span style={{ color: 'var(--fg)', minWidth: 0 }}>
                  <strong>{label}</strong>
                  {e.planet && e.city && (
                    <span style={{ color: 'var(--fg-muted)' }}>
                      {' · '}
                      {e.planet}
                    </span>
                  )}
                  {e.system && (
                    <span style={{ color: 'var(--fg-dim)', fontSize: 11 }}>
                      {' · '}
                      {e.system}
                    </span>
                  )}
                </span>
                <span
                  className="mono"
                  style={{
                    fontSize: 11,
                    color: 'var(--fg-dim)',
                    whiteSpace: 'nowrap',
                  }}
                >
                  {formatDwell(e.dwell_seconds)} · {e.visit_count} visit
                  {e.visit_count === 1 ? '' : 's'}
                </span>
              </div>
              <div
                style={{
                  height: 6,
                  borderRadius: 3,
                  background: 'var(--bg-elev)',
                  overflow: 'hidden',
                }}
              >
                <div
                  style={{
                    width: `${pct}%`,
                    height: '100%',
                    background: 'var(--accent)',
                    transition: 'width 200ms ease',
                  }}
                />
              </div>
            </li>
          );
        })}
      </ol>
      {entries.length > topN && (
        <div style={{ fontSize: 11, color: 'var(--fg-dim)' }}>
          + {entries.length - topN} more location
          {entries.length - topN === 1 ? '' : 's'} not shown
          {totalSec > 0 ? ` · top ${topN} dwell ${formatDwell(totalSec)}` : ''}
        </div>
      )}
    </div>
  );
}
