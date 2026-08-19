/**
 * Human-readable presentation metadata for every `GameEvent` variant.
 *
 * Source of truth for the underlying enum is
 * `crates/starstats-core/src/events.rs` — keep this dict in sync when
 * variants are added/renamed. The TS shape mirrors the serde
 * `#[serde(tag = "type", rename_all = "snake_case")]` discriminator,
 * so the keys here are exactly the strings the server emits.
 *
 * Each entry carries four facets so callers can compose displays:
 *   - `label`   – verb-first action phrase ("Killed someone")
 *   - `group`   – coarse category for filter sectioning / accents
 *   - `glyph`   – single-codepoint icon for chip/badge contexts
 *   - `accent`  – CSS variable for the left-border / chip background
 *
 * The fallback path title-cases unknown snake_case so a server-side
 * variant we haven't yet labelled still renders readably rather than
 * leaking the raw identifier.
 */

export type EventGroup =
  | 'combat'
  | 'travel'
  | 'loadout'
  | 'commerce'
  | 'mission'
  | 'vehicle'
  | 'session'
  | 'system';

export interface EventTypeMeta {
  /** Verb-first action phrase ("Killed someone", "Stowed ship"). */
  label: string;
  /** Coarse category for filter grouping and accent assignment. */
  group: EventGroup;
  /** Single-codepoint glyph for chip / badge / left-rail contexts. */
  glyph: string;
  /** CSS variable that callers can drop into borderColor /
   *  backgroundColor. Driven by `group` so all variants in the
   *  same category share an accent. */
  accent: string;
  /** Original raw identifier — handy for tooltips and analytics. */
  raw: string;
}

const GROUP_ACCENTS: Record<EventGroup, string> = {
  combat: 'var(--danger)',
  travel: 'var(--accent)',
  loadout: 'var(--info)',
  commerce: 'var(--ok)',
  mission: 'var(--ok)',
  vehicle: 'var(--info)',
  session: 'var(--fg-dim)',
  system: 'var(--border-strong)',
};

const GROUP_LABELS: Record<EventGroup, string> = {
  combat: 'Combat',
  travel: 'Travel',
  loadout: 'Loadout',
  commerce: 'Commerce',
  mission: 'Missions',
  vehicle: 'Vehicles',
  session: 'Session',
  system: 'System',
};

interface RawMetaEntry {
  label: string;
  group: EventGroup;
  glyph: string;
}

/** The full 30-variant catalogue. Order intentional — keeps related
 *  variants near each other so the diff stays readable when a new
 *  category is added. */
const META: Record<string, RawMetaEntry> = {
  // -- Session lifecycle --
  process_init: { label: 'Game started', group: 'session', glyph: '▶' },
  legacy_login: { label: 'Logged in', group: 'session', glyph: '🔑' },
  resolve_spawn: { label: 'Spawned', group: 'session', glyph: '✨' },
  session_end: { label: 'Session ended', group: 'session', glyph: '⏹' },

  // -- Travel & navigation --
  join_pu: { label: 'Joined PU', group: 'travel', glyph: '🌐' },
  change_server: { label: 'Changed server', group: 'travel', glyph: '🔀' },
  seed_solar_system: { label: 'Loaded system', group: 'travel', glyph: '✦' },
  quantum_target_selected: {
    label: 'Quantum jump',
    group: 'travel',
    glyph: '⚡',
  },
  quantum_route: {
    label: 'Plotted route',
    group: 'travel',
    glyph: '🧭',
  },
  planet_terrain_load: { label: 'Loaded planet', group: 'travel', glyph: '🪐' },

  // -- Combat --
  actor_death: { label: 'Killed someone', group: 'combat', glyph: '⚔' },
  player_death: { label: 'You died', group: 'combat', glyph: '💀' },
  player_incapacitated: {
    label: 'Incapacitated',
    group: 'combat',
    glyph: '🩹',
  },
  vehicle_destruction: {
    label: 'Vehicle destroyed',
    group: 'combat',
    glyph: '💥',
  },

  // -- Vehicle / loadout --
  vehicle_stowed: { label: 'Stowed ship', group: 'vehicle', glyph: '🚀' },
  attachment_received: {
    label: 'Attached gear',
    group: 'loadout',
    glyph: '🔧',
  },
  location_inventory_requested: {
    label: 'Opened inventory',
    group: 'loadout',
    glyph: '🎒',
  },
  item_equip_change: {
    label: 'Loadout changed',
    group: 'loadout',
    glyph: '🎽',
  },

  // -- Missions --
  mission_start: { label: 'Mission started', group: 'mission', glyph: '📜' },
  mission_end: { label: 'Mission ended', group: 'mission', glyph: '🏁' },
  mission_objective: {
    label: 'Mission objective',
    group: 'mission',
    glyph: '🎯',
  },

  // -- Commerce --
  shop_buy_request: { label: 'Shop purchase', group: 'commerce', glyph: '🛒' },
  shop_flow_response: {
    label: 'Shop response',
    group: 'commerce',
    glyph: '🏪',
  },
  commodity_buy_request: {
    label: 'Bought commodity',
    group: 'commerce',
    glyph: '🛒',
  },
  commodity_sell_request: {
    label: 'Sold commodity',
    group: 'commerce',
    glyph: '💰',
  },

  // -- System / instrumentation --
  hud_notification: { label: 'HUD notice', group: 'system', glyph: '🔔' },
  game_crash: { label: 'Game crashed', group: 'system', glyph: '💢' },
  launcher_activity: {
    label: 'Launcher activity',
    group: 'system',
    glyph: '🧭',
  },
  burst_summary: { label: 'Burst summary', group: 'system', glyph: '✦' },
  remote_match: { label: 'Remote rule', group: 'system', glyph: '📡' },
};

/** Look up presentation metadata for an event_type string. Unknown
 *  variants fall back to a title-cased version of the raw identifier,
 *  classified as `system`, so the UI degrades readably rather than
 *  leaking a snake_case discriminator. */
export function formatEventType(raw: string): EventTypeMeta {
  const meta = META[raw];
  if (meta) {
    return {
      ...meta,
      accent: GROUP_ACCENTS[meta.group],
      raw,
    };
  }
  return {
    label: titleCaseSnake(raw),
    group: 'system',
    glyph: '•',
    accent: GROUP_ACCENTS.system,
    raw,
  };
}

/** Display name for a group ("combat" → "Combat"). */
export function groupLabel(group: EventGroup): string {
  return GROUP_LABELS[group];
}

/** Order in which groups should appear in filter UI / breakdowns.
 *  Combat first because it's typically the most viewed slice; system
 *  last as it's instrumentation noise. */
export const GROUP_ORDER: ReadonlyArray<EventGroup> = [
  'combat',
  'travel',
  'vehicle',
  'loadout',
  'commerce',
  'mission',
  'session',
  'system',
];

function titleCaseSnake(s: string): string {
  if (!s) return '';
  return s
    .split('_')
    .filter((p) => p.length > 0)
    .map((p) => p[0].toUpperCase() + p.slice(1).toLowerCase())
    .join(' ');
}
