'use client';

import React from 'react';

/**
 * Column table for the verbose surfaces: sessions, orders, uplinks, records.
 * Lives inside a `Plane` — it brings no chrome of its own.
 *
 * A table is a `tilt="flat"` context by definition: reading across a row is
 * the job, and a 13° sheet fights that. Never put one on a tilted plane.
 */
export interface HoloColumn<R> {
  key: keyof R & string;
  label: React.ReactNode;
  /** Right-aligns and sets tabular figures. */
  numeric?: boolean;
  width?: string | number;
}

export function HoloTable<R extends { key?: string | number }>({
  columns = [],
  rows = [],
  onRowClick,
}: {
  columns?: HoloColumn<R>[];
  rows?: R[];
  onRowClick?: (row: R) => void;
}) {
  return (
    <table className="hp-tbl">
      <thead>
        <tr>
          {columns.map((c) => (
            <th
              key={c.key}
              style={c.width ? { width: c.width } : undefined}
              scope="col"
            >
              {c.label}
            </th>
          ))}
        </tr>
      </thead>
      <tbody>
        {rows.map((r, i) => (
          <tr
            key={r.key ?? i}
            onClick={onRowClick ? () => onRowClick(r) : undefined}
            style={onRowClick ? { cursor: 'pointer' } : undefined}
          >
            {columns.map((c) => (
              <td key={c.key} className={c.numeric ? 'v' : undefined}>
                {r[c.key] as React.ReactNode}
              </td>
            ))}
          </tr>
        ))}
      </tbody>
    </table>
  );
}

export interface HoloKVItem {
  k: React.ReactNode;
  v: React.ReactNode;
}

/** Field list — the densest way to state facts. */
export function HoloKV({ items = [] }: { items?: HoloKVItem[] }) {
  return (
    <dl className="hp-kv">
      {items.map((it, i) => (
        <React.Fragment key={i}>
          <dt>{it.k}</dt>
          <dd>{it.v}</dd>
        </React.Fragment>
      ))}
    </dl>
  );
}
