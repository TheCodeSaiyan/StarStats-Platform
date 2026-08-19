import type { ReferenceCategory } from './reference-types';
import type { Quantiles } from './kb-stats';
import { buildStatRow, type StatRow, type Units } from './kb-viz';
import { KB_GROUP_SPECS, readNumber } from './kb-detail';
import type { RadarAxis } from '@/components/kb/HandlingRadar';

export interface VisualGroup {
  title: string;
  rows: StatRow[];
}

export interface VisualModel {
  groups: VisualGroup[];
  radarAxes: RadarAxis[];
  /** Up to 4 pinned headline rows (Phase 2 makes the selection user-driven). */
  headline: StatRow[];
}

/** Default radar axis metric paths per category. Only categories whose
 *  data supports a capability radar (all-higher-is-better axes) get one;
 *  items + locations are intentionally absent (no radar — the
 *  `radarAxes.length >= 3` gate in the view then renders none). */
const RADAR_PATHS: Partial<Record<ReferenceCategory, string[]>> = {
  vehicle: ['speed.scm', 'agility.roll', 'agility.yaw', 'weaponry.fixed_weapons.dps_total', 'health', 'shield_hp'],
  weapon: [
    'personal_weapon.damage.dps_total',
    'personal_weapon.damage.alpha_total',
    'personal_weapon.rof',
    'personal_weapon.effective_range',
    'personal_weapon.ammunition.speed',
  ],
};

const HEADLINE_PATHS: Partial<Record<ReferenceCategory, Array<{ path: string; label: string; unit: string }>>> = {
  vehicle: [
    { path: 'speed.scm', label: 'SCM speed', unit: 'm/s' },
    { path: 'agility.roll', label: 'Roll rate', unit: '°/s' },
    { path: 'health', label: 'Hull HP', unit: 'hp' },
    { path: 'weaponry.fixed_weapons.dps_total', label: 'Pilot DPS', unit: 'dps' },
  ],
  weapon: [
    { path: 'personal_weapon.damage.dps_total', label: 'DPS', unit: 'dps' },
    { path: 'personal_weapon.damage.alpha_total', label: 'Alpha', unit: 'dmg' },
    { path: 'personal_weapon.rof', label: 'Fire rate', unit: 'rpm' },
    { path: 'personal_weapon.effective_range', label: 'Range', unit: 'm' },
  ],
  item: [
    // Armor HP leads when present (armor items); drops to physical for
    // everything else since the headline filters absent paths.
    { path: 'durability.health', label: 'Armor HP', unit: 'hp' },
    { path: 'mass', label: 'Mass', unit: 'kg' },
    { path: 'dimension.volume_converted', label: 'Volume', unit: 'µSCU' },
    { path: 'uex_prices.purchase', label: 'Buy price', unit: 'aUEC' },
  ],
  location: [
    { path: 'child_count', label: 'Sub-locations', unit: '' },
    { path: 'mission_count', label: 'Missions', unit: '' },
    { path: 'size', label: 'Diameter', unit: 'm' },
  ],
};

const SHORT_LABEL: Record<string, string> = {
  'speed.scm': 'Speed',
  'agility.roll': 'Roll',
  'agility.yaw': 'Yaw',
  'weaponry.fixed_weapons.dps_total': 'Firepower',
  health: 'Hull',
  shield_hp: 'Shield',
  // weapon radar axes
  'personal_weapon.damage.dps_total': 'DPS',
  'personal_weapon.damage.alpha_total': 'Alpha',
  'personal_weapon.rof': 'RoF',
  'personal_weapon.effective_range': 'Range',
  'personal_weapon.ammunition.speed': 'Speed',
};

export function buildVisualModel(
  category: ReferenceCategory,
  metadata: Record<string, unknown>,
  bucket: Record<string, Quantiles>,
  units: Units,
): VisualModel {
  const specs = KB_GROUP_SPECS[category] ?? [];
  const groups: VisualGroup[] = [];
  for (const spec of specs) {
    const rows: StatRow[] = [];
    for (const f of spec.fields) {
      const v = readNumber(metadata, f.path);
      if (v === undefined) continue;
      rows.push(buildStatRow(f.label, f.unit ?? '', v, bucket[f.path], units));
    }
    if (rows.length > 0) groups.push({ title: spec.title, rows });
  }

  const radarAxes: RadarAxis[] = (RADAR_PATHS[category] ?? [])
    .map((path) => {
      const v = readNumber(metadata, path);
      const qd = bucket[path];
      if (v === undefined || !qd || qd.n < 5) return null;
      return { label: SHORT_LABEL[path] ?? path, value: v, q: qd };
    })
    .filter((a): a is RadarAxis => a !== null);

  const headline: StatRow[] = (HEADLINE_PATHS[category] ?? [])
    .map(({ path, label, unit }) => {
      const v = readNumber(metadata, path);
      return v === undefined ? null : buildStatRow(label, unit, v, bucket[path], units);
    })
    .filter((r): r is StatRow => r !== null)
    .slice(0, 4);

  return { groups, radarAxes, headline };
}
