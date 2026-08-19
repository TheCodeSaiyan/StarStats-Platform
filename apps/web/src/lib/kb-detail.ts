/**
 * Meaningful presentation of the rich wiki `metadata` blob on KB detail
 * pages. Rather than dump (or hide) ~70 nested fields, we curate the
 * useful ones into themed groups via a per-category declarative spec.
 *
 * The wiki schema is consistent per category, so a spec of
 * `group -> [{ path, label, unit?, kind? }]` (dotted paths into the
 * metadata) lets us render scannable sections and drop the bookkeeping
 * noise (uuid, link, web_url, images, raw component arrays). Fields not
 * in a spec simply don't render; groups with no present fields are
 * dropped. Anything genuinely useful but un-curated still lives in the
 * raw escape hatch on the page.
 *
 * `metadata` is opaque (`Record<string, unknown>`) — every read is
 * defensive at the boundary.
 */

import type { ReferenceCategory } from './reference';

type FieldKind = 'number' | 'text' | 'localized' | 'bool' | 'count' | 'money';

export interface FieldSpec {
  /** Dotted path into the metadata object, e.g. `speed.scm`. */
  path: string;
  label: string;
  unit?: string;
  kind?: FieldKind;
}

export interface GroupSpec {
  title: string;
  fields: FieldSpec[];
}

export interface DetailRow {
  label: string;
  value: string;
}

export interface DetailGroup {
  title: string;
  rows: DetailRow[];
}

// -- value access + formatting --------------------------------------

function getPath(obj: Record<string, unknown>, path: string): unknown {
  let cur: unknown = obj;
  for (const seg of path.split('.')) {
    if (cur === null || typeof cur !== 'object') return undefined;
    cur = (cur as Record<string, unknown>)[seg];
  }
  return cur;
}

/** Pick a human string from a localized object (`{en_EN, de_DE, …}`) or
 *  a plain string. Returns undefined for anything else / empty. */
function localized(v: unknown): string | undefined {
  if (typeof v === 'string') return v.trim() || undefined;
  if (v && typeof v === 'object') {
    const o = v as Record<string, unknown>;
    const s = o.en_EN ?? o.en ?? o.name;
    if (typeof s === 'string') return s.trim() || undefined;
  }
  return undefined;
}

function fmtNumber(n: number, unit?: string): string {
  // Trim to at most 2 decimals; thousands separators for big numbers.
  const rounded = Math.abs(n) >= 100 ? Math.round(n) : Math.round(n * 100) / 100;
  const s = rounded.toLocaleString('en-US');
  return unit ? `${s} ${unit}` : s;
}

function formatValue(raw: unknown, field: FieldSpec): string | undefined {
  const kind = field.kind ?? 'number';
  switch (kind) {
    case 'count': {
      if (Array.isArray(raw)) return raw.length > 0 ? String(raw.length) : undefined;
      return undefined;
    }
    case 'bool':
      return typeof raw === 'boolean' ? (raw ? 'Yes' : 'No') : undefined;
    case 'text':
      return typeof raw === 'string' && raw.trim() ? raw.trim() : undefined;
    case 'localized':
      return localized(raw);
    case 'money': {
      const n = typeof raw === 'number' ? raw : Number(raw);
      if (!Number.isFinite(n) || n <= 0) return undefined;
      return field.unit ? `${n.toLocaleString('en-US')} ${field.unit}` : `$${n.toLocaleString('en-US')}`;
    }
    case 'number':
    default: {
      const n = typeof raw === 'number' ? raw : Number(raw);
      if (!Number.isFinite(n)) return undefined;
      // Drop pure zeros — almost always "not applicable" noise on these
      // wiki records (cargo on a fighter, etc.), and zero-rows bloat the
      // grid without informing.
      if (n === 0) return undefined;
      return fmtNumber(n, field.unit);
    }
  }
}

// -- per-category specs ---------------------------------------------
//
// Vehicle headline specs (dimensions, scm/afterburner speed, crew,
// cargo, mass, production status, description) already render in the
// Ship Matrix section, so they're deliberately omitted here to avoid
// duplication — these groups add the depth the Ship Matrix doesn't.

const VEHICLE_GROUPS: GroupSpec[] = [
  {
    title: 'Flight & handling',
    fields: [
      { path: 'speed.scm', label: 'SCM speed', unit: 'm/s' },
      { path: 'speed.max', label: 'Max speed', unit: 'm/s' },
      { path: 'speed.boost_forward', label: 'Boost (fwd)', unit: 'm/s' },
      { path: 'agility.pitch', label: 'Pitch', unit: '°/s' },
      { path: 'agility.yaw', label: 'Yaw', unit: '°/s' },
      { path: 'agility.roll', label: 'Roll', unit: '°/s' },
      { path: 'agility.acceleration', label: 'Acceleration', unit: 'm/s²' },
    ],
  },
  {
    title: 'Quantum travel',
    fields: [
      { path: 'quantum.quantum_speed', label: 'Quantum speed', unit: 'm/s' },
      { path: 'quantum.quantum_range', label: 'Range', unit: 'Gm' },
      { path: 'quantum.quantum_fuel_capacity', label: 'Q-fuel', unit: 'L' },
      { path: 'quantum.quantum_spool_time', label: 'Spool time', unit: 's' },
    ],
  },
  {
    title: 'Survivability',
    fields: [
      { path: 'health', label: 'Hull HP', unit: 'hp' },
      { path: 'shield_hp', label: 'Shield HP', unit: 'hp' },
      { path: 'shield.regeneration', label: 'Shield regen', unit: 'hp/s' },
      { path: 'armor.deflection', label: 'Armor deflection' },
      { path: 'damage_limits.before_destruction', label: 'Damage to destroy', unit: 'hp' },
    ],
  },
  {
    title: 'Power & fuel',
    fields: [
      { path: 'fuel.capacity', label: 'Fuel capacity', unit: 'L' },
      { path: 'fuel.intake_rate', label: 'Fuel intake', unit: 'L/s' },
      { path: 'propulsion.thrust_capacity', label: 'Thrust capacity', unit: 'N' },
      { path: 'propulsion.thrusters', label: 'Thrusters', kind: 'count' },
    ],
  },
  {
    title: 'Crew & interior',
    fields: [
      { path: 'crew.min', label: 'Min crew' },
      { path: 'crew.max', label: 'Max crew' },
      { path: 'seating.crew_stations', label: 'Crew stations' },
      { path: 'seating.beds', label: 'Beds' },
      { path: 'seating.escape_pods', label: 'Escape pods' },
      { path: 'seating.medical_beds', label: 'Medical beds' },
    ],
  },
  {
    title: 'Weaponry',
    fields: [
      { path: 'weaponry.fixed_weapons.dps_total', label: 'Pilot DPS', unit: 'dps' },
      { path: 'weaponry.fixed_weapons.alpha_total', label: 'Pilot alpha', unit: 'dmg' },
      { path: 'weaponry.fixed_weapons.sustained_dps_total', label: 'Sustained DPS', unit: 'dps' },
      { path: 'turrets.manned', label: 'Manned turrets', kind: 'count' },
      { path: 'turrets.remote', label: 'Remote turrets', kind: 'count' },
      { path: 'ports', label: 'Hardpoints', kind: 'count' },
    ],
  },
  {
    title: 'Mass & signature',
    fields: [
      { path: 'mass', label: 'Mass', unit: 'kg' },
      { path: 'mass_total', label: 'Mass (loaded)', unit: 'kg' },
      { path: 'emission.em_idle', label: 'EM (idle)' },
      { path: 'emission.em_max', label: 'EM (max)' },
      { path: 'emission.ir', label: 'IR signature' },
    ],
  },
  {
    title: 'Acquisition',
    fields: [
      { path: 'msrp', label: 'Pledge price', kind: 'money' },
      { path: 'uex_prices.purchase', label: 'In-game buy', unit: 'aUEC', kind: 'money' },
      { path: 'insurance.claim_time', label: 'Insurance claim', unit: 'min' },
      { path: 'production_status', label: 'Production status', kind: 'localized' },
    ],
  },
];

const WEAPON_GROUPS: GroupSpec[] = [
  {
    title: 'Damage & fire',
    fields: [
      { path: 'personal_weapon.damage.dps_total', label: 'DPS', unit: 'dps' },
      { path: 'personal_weapon.damage.alpha_total', label: 'Alpha damage', unit: 'dmg' },
      { path: 'personal_weapon.rof', label: 'Fire rate', unit: 'rpm' },
      { path: 'personal_weapon.pellets_per_shot', label: 'Pellets/shot' },
    ],
  },
  {
    title: 'Range & handling',
    fields: [
      // `magazine_size` / `capacity` are deliberately excluded — the wiki
      // uses a 99999 sentinel for melee/infinite, which poisons quantiles.
      { path: 'personal_weapon.effective_range', label: 'Effective range', unit: 'm' },
      { path: 'personal_weapon.ammunition.speed', label: 'Projectile speed', unit: 'm/s' },
      { path: 'personal_weapon.spread.max', label: 'Max spread', unit: '°' },
    ],
  },
  {
    title: 'Classification',
    fields: [
      { path: 'type_label', label: 'Type', kind: 'text' },
      { path: 'sub_type_label', label: 'Subtype', kind: 'text' },
      { path: 'classification_label', label: 'Class', kind: 'text' },
      { path: 'rarity', label: 'Rarity', kind: 'text' },
    ],
  },
  {
    title: 'Physical',
    fields: [
      { path: 'mass', label: 'Mass', unit: 'kg' },
      { path: 'size', label: 'Size' },
      { path: 'dimension.length', label: 'Length', unit: 'm' },
      { path: 'dimension.width', label: 'Width', unit: 'm' },
      { path: 'dimension.height', label: 'Height', unit: 'm' },
      { path: 'dimension.volume', label: 'Volume', unit: 'µSCU' },
    ],
  },
  {
    title: 'Acquisition',
    fields: [{ path: 'uex_prices.purchase', label: 'In-game buy', unit: 'aUEC', kind: 'money' }],
  },
];

const ITEM_GROUPS: GroupSpec[] = [
  {
    // Armor-only — non-armor items carry none of these, so the group is
    // dropped (empty groups don't render). `armor.damage_multiplier.*` is
    // a "damage taken" multiplier (<1 = more protective).
    title: 'Protection',
    fields: [
      { path: 'durability.health', label: 'Health', unit: 'hp' },
      { path: 'armor.deflection.physical', label: 'Deflection (phys)' },
      { path: 'armor.deflection.energy', label: 'Deflection (energy)' },
      { path: 'armor.damage_multiplier.physical', label: 'Phys dmg taken' },
      { path: 'armor.damage_multiplier.energy', label: 'Energy dmg taken' },
    ],
  },
  {
    title: 'Classification',
    fields: [
      { path: 'type_label', label: 'Type', kind: 'text' },
      { path: 'sub_type_label', label: 'Subtype', kind: 'text' },
    ],
  },
  {
    title: 'Physical',
    fields: [
      { path: 'mass', label: 'Mass', unit: 'kg' },
      { path: 'size', label: 'Size' },
      { path: 'dimension.length', label: 'Length', unit: 'm' },
      { path: 'dimension.width', label: 'Width', unit: 'm' },
      { path: 'dimension.height', label: 'Height', unit: 'm' },
      { path: 'dimension.volume', label: 'Volume', unit: 'µSCU' },
    ],
  },
  {
    title: 'Acquisition',
    fields: [{ path: 'uex_prices.purchase', label: 'In-game buy', unit: 'aUEC', kind: 'money' }],
  },
];

const LOCATION_GROUPS: GroupSpec[] = [
  {
    title: 'Overview',
    fields: [
      { path: 'system', label: 'System', kind: 'text' },
      { path: 'designation', label: 'Designation', kind: 'text' },
      { path: 'type.name', label: 'Type', kind: 'text' },
      { path: 'type.classification', label: 'Classification', kind: 'text' },
      { path: 'parent.name', label: 'Orbits', kind: 'text' },
      { path: 'star.name', label: 'Star', kind: 'text' },
      { path: 'affiliation', label: 'Affiliation', kind: 'text' },
      { path: 'jurisdiction', label: 'Jurisdiction', kind: 'text' },
    ],
  },
  {
    title: 'Profile',
    fields: [
      { path: 'size', label: 'Diameter', unit: 'm' },
      { path: 'child_count', label: 'Sub-locations' },
      { path: 'mission_count', label: 'Missions' },
      { path: 'has_resources', label: 'Has resources', kind: 'bool' },
      { path: 'is_scannable', label: 'Scannable', kind: 'bool' },
      { path: 'respawn_location_type', label: 'Respawn', kind: 'text' },
      { path: 'amenities', label: 'Amenities', kind: 'count' },
    ],
  },
];

const GROUP_SPECS: Record<ReferenceCategory, GroupSpec[]> = {
  vehicle: VEHICLE_GROUPS,
  weapon: WEAPON_GROUPS,
  item: ITEM_GROUPS,
  location: LOCATION_GROUPS,
};

/**
 * Build the curated, grouped detail sections for a category from its raw
 * metadata. Drops empty fields and empty groups, so the page only shows
 * sections with real content.
 */
export function buildDetailGroups(
  category: ReferenceCategory,
  metadata: Record<string, unknown>,
): DetailGroup[] {
  const specs = GROUP_SPECS[category] ?? [];
  const groups: DetailGroup[] = [];
  for (const spec of specs) {
    const rows: DetailRow[] = [];
    for (const field of spec.fields) {
      const value = formatValue(getPath(metadata, field.path), field);
      if (value !== undefined) rows.push({ label: field.label, value });
    }
    if (rows.length > 0) groups.push({ title: spec.title, rows });
  }
  return groups;
}

// Re-exported for the visual view model (kb-view-model.ts), which reuses
// the same per-category group/field specs and the dotted-path reader.
export const KB_GROUP_SPECS = GROUP_SPECS;

/** Read a numeric leaf at `path` from the metadata, or undefined. */
export function readNumber(metadata: Record<string, unknown>, path: string): number | undefined {
  const v = getPath(metadata, path);
  return typeof v === 'number' && Number.isFinite(v) ? v : undefined;
}
