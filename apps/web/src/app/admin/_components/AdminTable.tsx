/**
 * Column-driven table for /admin listings.
 *
 * NOW THE SYSTEM'S TABLE. It used to carry its own inline styles — a
 * `--bg-elev` header band, 1px `--border` rules, 11px semibold caps — which is
 * the flat idiom, and inline, so no stylesheet could correct it. It emits
 * `hp-tbl` markup instead: the same element structure and the same classes
 * `HoloTable` produces, so every admin listing is drawn by the system's own
 * rules and follows the calibration.
 *
 * WHY NOT JUST USE `HoloTable`. Two reasons, both about the callers:
 *
 *   - Its API is `rows` of plain objects keyed by column key. This one's cells
 *     are RENDER FUNCTIONS, and every admin listing uses that to put chips,
 *     links and forms inside a cell. Converting the API would mean rewriting
 *     seven call sites to marshal ReactNodes into a row object, for no gain.
 *   - `HoloTable` is `'use client'`. Admin listings are server-rendered, and
 *     making them client components to get a table would ship the whole
 *     listing to the browser.
 *
 * So the markup is shared and the API is not. If `hp-tbl`'s structure ever
 * changes, this file has to follow it — that is the cost, and it is smaller
 * than the alternative.
 */

// Explicit React import: this repo's vitest uses the classic JSX
// runtime, so a JSX-rendering component ReferenceErrors without it.
import React from 'react';
import { Flatline } from 'holo';

export interface AdminTableColumn<T> {
  readonly header: string;
  readonly cell: (row: T) => React.ReactNode;
  /** Right-aligns and sets tabular figures, as `HoloColumn.numeric` does. */
  readonly numeric?: boolean;
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
    // The empty state replaces the table entirely rather than rendering an
    // empty <tbody>. `Flatline` is the system's own no-data state, so an empty
    // admin listing reads the same as an empty listing anywhere else.
    return <Flatline compact reason="no-data" title={empty} />;
  }

  return (
    <table className="hp-tbl">
      <thead>
        <tr>
          {columns.map((c) => (
            <th key={c.header} scope="col">
              {c.header}
            </th>
          ))}
        </tr>
      </thead>
      <tbody>
        {rows.map((row) => (
          <tr key={rowKey(row)}>
            {columns.map((c) => (
              <td key={c.header} className={c.numeric ? 'v' : undefined}>
                {c.cell(row)}
              </td>
            ))}
          </tr>
        ))}
      </tbody>
    </table>
  );
}
