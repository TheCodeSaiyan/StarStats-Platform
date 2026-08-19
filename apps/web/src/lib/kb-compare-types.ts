/**
 * Client-safe types + pure model builders for the multi-ship comparison
 * view. No `server-only` import — these run in the client `KbDetailView`.
 * Reuses the metric group specs from `kb-detail` and `kb-viz`'s value
 * formatting.
 */

import type { ReferenceCategory } from './reference-types';
import { KB_GROUP_SPECS } from './kb-detail';
import { buildStatRow, type Units } from './kb-viz';

export interface CompareEntry {
  slug: string;
  class_name: string;
  display_name: string;
  peer_group: string;
  metrics: Record<string, number>;
}

export interface CompareResponse {
  entries: CompareEntry[];
}

export interface SortSpec {
  key: string;
  dir: 'asc' | 'desc';
}

export interface MatrixCell {
  value: number | null;
  text: string;
  /** 0–100 across the selected set for this row; null when value absent. */
  fillPct: number | null;
  isLeader: boolean;
}

export interface MatrixRow {
  key: string;
  label: string;
  unit: string;
  group: string;
  cells: MatrixCell[]; // aligned to `columns` order
}

export interface ComparisonMatrix {
  columns: CompareEntry[]; // anchor first, then sorted others
  rows: MatrixRow[];
}

/** Flatten the per-category group specs into ordered numeric metric defs. */
function metricDefs(
  category: ReferenceCategory,
): Array<{ key: string; label: string; unit: string; group: string }> {
  const specs = KB_GROUP_SPECS[category] ?? [];
  const out: Array<{ key: string; label: string; unit: string; group: string }> = [];
  for (const g of specs) {
    for (const f of g.fields) {
      // Skip non-numeric fields; undefined kind defaults to 'number'
      if (f.kind !== undefined && f.kind !== 'number') continue;
      out.push({ key: f.path, label: f.label, unit: f.unit ?? '', group: g.title });
    }
  }
  return out;
}

function formatValue(unit: string, value: number, units: Units): string {
  return buildStatRow('', unit, value, undefined, units).valueText;
}

export function buildComparisonMatrix(
  category: ReferenceCategory,
  anchor: CompareEntry,
  others: CompareEntry[],
  units: Units,
  sort: SortSpec,
): ComparisonMatrix {
  const sorted = [...others].sort((a, b) => {
    const an = a.metrics[sort.key] ?? -Infinity;
    const bn = b.metrics[sort.key] ?? -Infinity;
    return sort.dir === 'desc' ? bn - an : an - bn;
  });
  const columns = [anchor, ...sorted];
  const rows: MatrixRow[] = [];

  for (const def of metricDefs(category)) {
    const raw = columns.map((c) =>
      typeof c.metrics[def.key] === 'number' ? c.metrics[def.key] : null,
    );
    const present = raw.filter((v): v is number => typeof v === 'number');
    if (present.length === 0) continue;

    const min = Math.min(...present);
    const max = Math.max(...present);

    const cells: MatrixCell[] = raw.map((v) => ({
      value: v,
      text: v === null ? '—' : formatValue(def.unit, v, units),
      fillPct: v === null ? null : max === min ? 100 : ((v - min) / (max - min)) * 100,
      isLeader: v !== null && present.length > 1 && v === max,
    }));

    rows.push({ key: def.key, label: def.label, unit: def.unit, group: def.group, cells });
  }

  return { columns, rows };
}

// -- Radar -----------------------------------------------------------

export interface RadarSeries {
  slug: string;
  name: string;
  values: number[]; // 0..1 fraction per axis, aligned to `axes`
}

export interface ComparisonRadar {
  axes: string[];
  series: RadarSeries[];
}

/** Per-axis fractions scaled to the selected set's min–max (floor 0.06). */
export function buildComparisonRadar(ships: CompareEntry[], axes: string[]): ComparisonRadar {
  const ranges = axes.map((k) => {
    const vals = ships
      .map((s) => s.metrics[k])
      .filter((v): v is number => typeof v === 'number');
    return {
      min: vals.length ? Math.min(...vals) : 0,
      max: vals.length ? Math.max(...vals) : 1,
    };
  });

  const series: RadarSeries[] = ships.map((s) => ({
    slug: s.slug,
    name: s.display_name,
    values: axes.map((k, i) => {
      const v = s.metrics[k];
      if (typeof v !== 'number') return 0.06;
      const { min, max } = ranges[i];
      return max === min ? 0.5 : Math.max(0.06, Math.min(1, (v - min) / (max - min)));
    }),
  }));

  return { axes, series };
}

// -- Leaderboard -----------------------------------------------------

export interface LeaderCard {
  key: string;
  label: string;
  valueText: string;
  winnerName: string;
}

/** Per-category superlatives (max wins for all entries). */
const SUPERLATIVES: Partial<
  Record<ReferenceCategory, Array<{ key: string; label: string; unit: string }>>
> = {
  vehicle: [
    { key: 'speed.scm', label: 'Fastest (SCM)', unit: 'm/s' },
    { key: 'health', label: 'Toughest hull', unit: 'hp' },
    { key: 'weaponry.fixed_weapons.dps_total', label: 'Most firepower', unit: 'dps' },
    { key: 'shield_hp', label: 'Strongest shield', unit: 'hp' },
  ],
  weapon: [
    { key: 'personal_weapon.damage.dps_total', label: 'Highest DPS', unit: 'dps' },
    { key: 'personal_weapon.damage.alpha_total', label: 'Biggest alpha', unit: 'dmg' },
    { key: 'personal_weapon.effective_range', label: 'Longest range', unit: 'm' },
    { key: 'personal_weapon.rof', label: 'Fastest fire rate', unit: 'rpm' },
  ],
  item: [
    { key: 'durability.health', label: 'Toughest armor', unit: 'hp' },
    { key: 'mass', label: 'Heaviest', unit: 'kg' },
    { key: 'dimension.volume_converted', label: 'Largest volume', unit: 'µSCU' },
  ],
  location: [
    { key: 'mission_count', label: 'Most missions', unit: '' },
    { key: 'child_count', label: 'Most sub-locations', unit: '' },
    { key: 'size', label: 'Largest', unit: 'm' },
  ],
};

export function buildLeaderboard(
  category: ReferenceCategory,
  ships: CompareEntry[],
  units: Units,
): LeaderCard[] {
  const out: LeaderCard[] = [];
  for (const s of SUPERLATIVES[category] ?? []) {
    let best: CompareEntry | undefined;
    let bestVal = -Infinity;
    for (const ship of ships) {
      const v = ship.metrics[s.key];
      if (typeof v === 'number' && v > bestVal) {
        bestVal = v;
        best = ship;
      }
    }
    if (!best) continue;
    out.push({
      key: s.key,
      label: s.label,
      valueText: formatValue(s.unit, bestVal, units),
      winnerName: best.display_name,
    });
  }
  return out;
}
