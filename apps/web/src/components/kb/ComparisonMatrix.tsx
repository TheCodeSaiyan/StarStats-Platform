import React from 'react';
import type { ComparisonMatrix as MatrixModel, SortSpec } from '@/lib/kb-compare-types';

/**
 * Sortable comparison matrix. Anchor column (index 0) is pinned + tinted.
 * Click a metric row label to sort the non-anchor columns by it. Each
 * cell shows the value + a mini bar normalised across the row.
 */
export function ComparisonMatrix({
  model,
  sort,
  onSort,
}: {
  model: MatrixModel;
  sort: SortSpec;
  onSort: (key: string) => void;
}) {
  const anchorTint = 'rgba(232,162,60,0.06)';
  return (
    <div style={{ overflowX: 'auto' }}>
      <table style={{ width: '100%', borderCollapse: 'collapse', fontSize: 12 }}>
        <thead>
          <tr>
            <th style={{ textAlign: 'left', padding: '9px 12px', color: 'var(--fg-muted)' }}>Metric</th>
            {model.columns.map((cEntry, i) => (
              <th
                key={cEntry.slug}
                style={{
                  textAlign: 'left', padding: '9px 12px', whiteSpace: 'nowrap', verticalAlign: 'bottom',
                  color: 'var(--fg)', fontWeight: 600,
                  background: i === 0 ? anchorTint : undefined,
                }}
              >
                {cEntry.display_name}
                {i === 0 && <div style={{ fontSize: 9, letterSpacing: '.08em', color: 'var(--accent)' }}>ANCHOR</div>}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {model.rows.map((row) => {
            const active = sort.key === row.key;
            return (
              <tr key={row.key} style={{ borderTop: '1px solid rgba(255,255,255,.05)' }}>
                <td style={{ padding: '9px 12px', whiteSpace: 'nowrap' }}>
                  <button
                    type="button"
                    aria-label={`Sort by ${row.label}`}
                    onClick={() => onSort(row.key)}
                    style={{ background: 'none', border: 'none', cursor: 'pointer', color: active ? 'var(--accent)' : 'var(--fg-muted)', fontSize: 12, padding: 0 }}
                  >
                    {row.label}{active ? (sort.dir === 'desc' ? ' ▼' : ' ▲') : ''}
                  </button>
                </td>
                {row.cells.map((cell, i) => (
                  <td key={i} style={{ padding: '9px 12px', minWidth: 96, background: i === 0 ? anchorTint : undefined }}>
                    <div style={{ color: cell.isLeader ? 'var(--accent)' : 'var(--fg)', fontWeight: cell.isLeader ? 650 : 400 }}>
                      {cell.text}{cell.isLeader && <span aria-hidden="true"> ◆</span>}
                    </div>
                    {cell.fillPct !== null && (
                      <div style={{ height: 5, background: 'var(--surface-2, #221d2b)', borderRadius: 3, marginTop: 5, overflow: 'hidden' }}>
                        <div style={{ height: '100%', width: `${cell.fillPct}%`, borderRadius: 3, background: i === 0 ? 'var(--accent, #E8A23C)' : 'rgba(255,255,255,.32)' }} />
                      </div>
                    )}
                  </td>
                ))}
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}
