/**
 * System breakdown — the travel "route map" as a system → location tree.
 *
 * Rather than plot fake spatial coordinates, this leans on the taxonomy
 * StarStats already resolves: group the trace's stops by SYSTEM, then by
 * location (city, else planet, else system), summing arrivals. Honest,
 * legible at any stop count, and it answers "where do I actually spend
 * time" better than a node scatter. Server-rendered, deterministic.
 */

import React from 'react';
import type { TraceEntry } from '@/lib/api';
import type { ReferenceCatalog } from '@/lib/reference';
import { EntityLink } from '@/components/kb/EntityLink';

const MAX_SYSTEMS = 4;
const MAX_LOCS = 5;

interface Loc {
  label: string;
  sublabel: string | null;
  visits: number;
}
interface Sys {
  system: string;
  total: number;
  locs: Loc[];
}

function aggregate(entries: TraceEntry[]): Sys[] {
  const bySystem = new Map<string, Map<string, Loc>>();
  for (const e of entries) {
    const system = (e.system ?? '').trim() || 'Unknown';
    const label = (e.city ?? e.planet ?? e.system ?? 'In transit').trim();
    if (!label) continue;
    // Show the planet as context only when the primary label is a city.
    const sublabel = e.city && e.planet ? e.planet : null;
    const locs = bySystem.get(system) ?? new Map<string, Loc>();
    const cur = locs.get(label) ?? { label, sublabel, visits: 0 };
    cur.visits += e.event_count;
    locs.set(label, cur);
    bySystem.set(system, locs);
  }
  return [...bySystem.entries()]
    .map(([system, locs]) => {
      const list = [...locs.values()].sort((a, b) => b.visits - a.visits);
      return { system, total: list.reduce((s, l) => s + l.visits, 0), locs: list };
    })
    .sort((a, b) => b.total - a.total)
    .slice(0, MAX_SYSTEMS);
}

export function SystemBreakdown({
  entries,
  catalog,
}: {
  entries: TraceEntry[];
  catalog: ReferenceCatalog;
}) {
  const systems = aggregate(entries);
  if (systems.length === 0) return null;
  const maxVisits = Math.max(...systems.flatMap((s) => s.locs.map((l) => l.visits)), 1);

  return (
    <div>
      {systems.map((s) => (
        <div key={s.system} className="sys">
          <div className="sys-hd">
            <span className="nm">{s.system}</span>
            <span className="ct">{s.total.toLocaleString()} visits</span>
          </div>
          {s.locs.slice(0, MAX_LOCS).map((l) => (
            <div key={l.label} className="loc">
              <span className="l">
                <EntityLink
                  category="location"
                  classKey={l.label}
                  catalog={catalog}
                  label={l.label}
                />
                {l.sublabel ? <small> · {l.sublabel}</small> : null}
              </span>
              <span className="n">{l.visits.toLocaleString()}</span>
              <span className="mtr">
                <i style={{ width: `${Math.round((l.visits / maxVisits) * 100)}%` }} />
              </span>
            </div>
          ))}
        </div>
      ))}
    </div>
  );
}
