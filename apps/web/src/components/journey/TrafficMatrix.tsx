/**
 * Traffic matrix — the transition graph as a stop × stop heat grid.
 *
 * Every ordered pair of stops is a cell; the darker the amber, the more
 * trips ran that leg (undirected, so the grid is symmetric). Capped to the
 * busiest stops so it stays legible. No layout geometry to tangle —
 * deterministic + server-rendered.
 */

import React from 'react';
import type { TransitionGraph } from './TransitionGraph';

const MAX_NODES = 7;

function truncate(s: string, n: number): string {
  return s.length > n ? s.slice(0, n - 1) + '…' : s;
}

export function TrafficMatrix({ graph }: { graph: TransitionGraph }) {
  // Keep the busiest stops so the matrix stays small + readable.
  const ranked = graph.nodes
    .map((nd, idx) => ({ idx, label: nd.label, visits: nd.visits }))
    .sort((a, b) => b.visits - a.visits)
    .slice(0, MAX_NODES);
  if (ranked.length < 2) return null;

  const posByIdx = new Map<number, number>();
  ranked.forEach((r, i) => posByIdx.set(r.idx, i));
  const size = ranked.length;

  const cells: number[][] = Array.from({ length: size }, () => new Array(size).fill(0));
  let maxCount = 1;
  for (const e of graph.edges) {
    const i = posByIdx.get(e.a);
    const j = posByIdx.get(e.b);
    if (i === undefined || j === undefined) continue;
    cells[i][j] = e.count;
    cells[j][i] = e.count;
    if (e.count > maxCount) maxCount = e.count;
  }

  const labels = ranked.map((r) => r.label);

  return (
    <div className="mtx">
      <table>
        <tbody>
          <tr>
            <th aria-hidden="true"></th>
            {labels.map((l) => (
              <th key={l} className="col" scope="col">
                {truncate(l, 12)}
              </th>
            ))}
          </tr>
          {labels.map((rowLabel, i) => (
            <tr key={rowLabel}>
              <th scope="row">{truncate(rowLabel, 14)}</th>
              {labels.map((colLabel, j) => {
                const v = cells[i][j];
                const bg =
                  i === j
                    ? 'var(--surface-2)'
                    : v === 0
                      ? 'rgba(255,230,200,0.05)'
                      : `rgba(232,162,60,${(0.12 + 0.78 * (v / maxCount)).toFixed(2)})`;
                return (
                  <td
                    key={j}
                    style={{ background: bg }}
                    title={i === j ? undefined : `${rowLabel} ↔ ${colLabel}: ${v} trips`}
                  />
                );
              })}
            </tr>
          ))}
        </tbody>
      </table>
      <div className="hud-note" style={{ marginTop: 8, textAlign: 'center' }}>
        Darker cell = more trips between row &amp; column
      </div>
    </div>
  );
}
