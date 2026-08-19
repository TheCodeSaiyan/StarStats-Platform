import React from 'react';
import { TONE_COLOR, type StatRow } from '@/lib/kb-viz';

/** Big pinned-metric callouts at the top of the visual view. */
export function HeadlineCallouts({ rows }: { rows: StatRow[] }) {
  if (rows.length === 0) return null;
  return (
    <div style={{ display: 'grid', gridTemplateColumns: `repeat(${Math.min(rows.length, 4)}, 1fr)`, gap: 12 }}>
      {rows.map((r) => (
        <div key={r.label} style={{ background: 'var(--surface, #16131d)', border: '1px solid var(--border, rgba(255,255,255,.07))', borderRadius: 10, padding: 12 }}>
          <div style={{ fontSize: 24, fontWeight: 650, color: 'var(--fg)' }}>{r.valueText}</div>
          <div style={{ fontSize: 10, textTransform: 'uppercase', letterSpacing: '.07em', color: 'var(--fg-muted)', marginTop: 4 }}>{r.label}</div>
          {r.band && <div style={{ fontSize: 11, marginTop: 5, fontWeight: 600, color: TONE_COLOR[r.band.tone] }}>{r.band.text}</div>}
        </div>
      ))}
    </div>
  );
}
