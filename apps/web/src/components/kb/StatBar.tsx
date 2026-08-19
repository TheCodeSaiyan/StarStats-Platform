import React from 'react';
import { TONE_COLOR, type StatRow } from '@/lib/kb-viz';

/**
 * One peer-relative stat: label + value, a min→max track with the
 * value dot + class-median tick, and a quantile band label. When the
 * row has no `fillPct` (stats missing) it degrades to label + value.
 */
export function StatBar({ row }: { row: StatRow }) {
  const hasTrack = row.fillPct !== undefined;
  return (
    <div style={{ marginBottom: 14 }}>
      <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'baseline', marginBottom: 5 }}>
        <span style={{ fontSize: 12, color: 'var(--fg-muted)' }}>{row.label}</span>
        <span style={{ fontSize: 13, fontWeight: 600, color: 'var(--fg)' }}>{row.valueText}</span>
      </div>
      {hasTrack && (
        <div style={{ position: 'relative', height: 7, background: 'var(--surface-2, #221d2b)', borderRadius: 4 }}>
          <div
            style={{
              position: 'absolute', left: 0, top: 0, bottom: 0,
              width: `${row.fillPct}%`,
              background: 'linear-gradient(90deg, rgba(232,162,60,.35), var(--accent, #E8A23C))',
              borderRadius: 4,
            }}
          />
          {row.medianPct !== undefined && (
            <div
              title="class median"
              style={{ position: 'absolute', top: -2, bottom: -2, left: `${row.medianPct}%`, width: 2, background: 'rgba(255,255,255,.5)', transform: 'translateX(-50%)' }}
            />
          )}
          <div
            style={{
              position: 'absolute', top: '50%', left: `${row.fillPct}%`,
              width: 11, height: 11, borderRadius: '50%',
              background: 'var(--accent, #E8A23C)', border: '2px solid var(--bg, #0F0E12)',
              transform: 'translate(-50%, -50%)',
            }}
          />
        </div>
      )}
      {row.band && (
        <div style={{ marginTop: 5, fontSize: 10.5, fontWeight: 600, color: TONE_COLOR[row.band.tone] }}>
          {row.band.text}
        </div>
      )}
    </div>
  );
}
