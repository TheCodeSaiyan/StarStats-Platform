/**
 * Column-driven table for /admin listings.
 *
 * Styles are lifted verbatim from the admin users list, which every
 * other admin table had independently re-implemented (each with its own
 * local `Th`). Cells are `ReactNode`, so callers keep rendering their
 * own chips, links and badges — this owns the chrome, not the content.
 *
 * The empty state replaces the table entirely rather than rendering an
 * empty <tbody>, matching what the users page did before extraction.
 */

// Explicit React import: this repo's vitest uses the classic JSX
// runtime, so a JSX-rendering component ReferenceErrors without it.
import React from 'react';

export interface AdminTableColumn<T> {
  readonly header: string;
  readonly cell: (row: T) => React.ReactNode;
}

export function AdminTable<T>({
  columns,
  rows,
  rowKey,
  empty,
}: {
  columns: readonly AdminTableColumn<T>[];
  rows: readonly T[];
  /** Stable per-row key — never the array index. */
  rowKey: (row: T) => string;
  empty: string;
}) {
  if (rows.length === 0) {
    return (
      <p
        style={{
          margin: 0,
          padding: '40px 24px',
          textAlign: 'center',
          color: 'var(--fg-muted)',
          fontSize: 14,
        }}
      >
        {empty}
      </p>
    );
  }

  return (
    <table style={{ width: '100%', borderCollapse: 'collapse', fontSize: 13 }}>
      <thead>
        <tr style={{ background: 'var(--bg-elev)' }}>
          {columns.map((c) => (
            <th
              key={c.header}
              style={{
                textAlign: 'left',
                padding: '10px 14px',
                fontWeight: 600,
                color: 'var(--fg-muted)',
                fontSize: 11,
                letterSpacing: '0.06em',
                textTransform: 'uppercase',
                borderBottom: '1px solid var(--border)',
              }}
            >
              {c.header}
            </th>
          ))}
        </tr>
      </thead>
      <tbody>
        {rows.map((row) => (
          <tr
            key={rowKey(row)}
            style={{ borderBottom: '1px solid var(--border)' }}
          >
            {columns.map((c) => (
              <td key={c.header} style={{ padding: '10px 14px' }}>
                {c.cell(row)}
              </td>
            ))}
          </tr>
        ))}
      </tbody>
    </table>
  );
}
