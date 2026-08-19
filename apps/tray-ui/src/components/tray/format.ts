/**
 * Display-formatting helpers shared across tray panes (Status, Logs).
 * Each function is pure and locale-aware where it makes sense; callers
 * should not need to think about NaN, negative, or missing values.
 */

import type { EventEnvelope } from 'api-client-ts';

export function fmtBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / 1024 / 1024).toFixed(2)} MB`;
}

export function fmtTime(iso: string | null | undefined): string {
  if (!iso) return '—';
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? iso : d.toLocaleTimeString();
}

export function fmtDate(iso: string): string {
  const d = new Date(iso);
  return Number.isNaN(d.getTime())
    ? iso
    : d.toLocaleDateString(undefined, { month: 'short', day: 'numeric' });
}

export function ageLabel(iso: string): string {
  const ms = Date.now() - new Date(iso).getTime();
  if (Number.isNaN(ms)) return iso;
  if (ms < 60_000) return `${Math.floor(ms / 1000)}s ago`;
  if (ms < 3_600_000) return `${Math.floor(ms / 60_000)}m ago`;
  if (ms < 86_400_000) return `${Math.floor(ms / 3_600_000)}h ago`;
  return `${Math.floor(ms / 86_400_000)}d ago`;
}

export function fmtCovPct(recognised: number, structuralOnly: number): string {
  const total = recognised + structuralOnly;
  return total === 0 ? '—' : `${((recognised / total) * 100).toFixed(1)}%`;
}

export type RowTone = 'ok' | 'warn' | 'danger' | 'accent' | 'info';

export const TONE_VAR: Record<RowTone, string> = {
  ok: 'var(--ok)',
  warn: 'var(--warn)',
  danger: 'var(--danger)',
  accent: 'var(--accent)',
  info: 'var(--info)',
};

/**
 * Maps an event_type to a tone. Shared by StatusPane's timeline and
 * LogsPane's grouped list so the same event paints the same colour
 * across panes.
 */
export function toneForType(eventType: string): RowTone {
  switch (eventType) {
    case 'actor_death':
    case 'vehicle_destruction':
      return 'danger';
    case 'legacy_login':
    case 'join_pu':
    case 'mission_completed':
      return 'ok';
    case 'quantum_target_selected':
      return 'accent';
    default:
      return 'info';
  }
}

/**
 * Human-readable verb fallback for an event_type. Used when the
 * server-side `summary` is empty (rare — unparseable payload) or when
 * a caller only has the event_type to work with. Keep this list
 * conservative: events not present here fall through to
 * `humanizeEventType()` which sentence-cases the snake_case key, so
 * the worst case is "Player death" rather than "player_death" —
 * still readable, never blocking.
 */
export const EVENT_VERB_TABLE: Record<string, string> = {
  player_death: 'Died',
  actor_death: 'Killed an actor',
  vehicle_destruction: 'Ship destroyed',
  quantum_target_selected: 'Set quantum course',
  location_changed: 'Moved to a new location',
  legacy_login: 'Logged in',
  join_pu: 'Joined Persistent Universe',
  mission_start: 'Mission started',
  mission_end: 'Mission ended',
  mission_completed: 'Mission completed',
  session_end: 'Session ended',
  game_crash: 'Game crashed',
  shop_buy_request: 'Bought items',
  commodity_buy_request: 'Bought commodities',
  commodity_sell_request: 'Sold commodities',
  vehicle_stowed: 'Vehicle stowed',
  hud_notification: 'HUD notification',
  burst_summary: 'Burst of activity',
};

/**
 * Flat lookup keyed by lowercased class_name → display_name. The
 * tray builds this once at App level from all four catalogues so
 * the prettifier doesn't need to know which category an event's
 * class_name belongs to — important because the Rust-side
 * `format_summary` doesn't distinguish vehicle / weapon / location
 * tokens in the rendered string.
 */
export type PrettyLookup = ReadonlyMap<string, string>;

/**
 * Replace any class-name-like tokens (`AEGS_Avenger_Stalker`,
 * `OOC_Stanton_2_Crusader`) in a Rust-formatted summary string with
 * their catalog display names. Tokens not present in the catalogue
 * are left as-is so the user always sees SOMETHING readable.
 *
 * Pattern: uppercase-led, contains at least one underscore. This is
 * conservative — short identifiers without underscores
 * (`Crusader`, `Hurston`) already render cleanly out of
 * `format_summary` and don't need replacement, while heavily-
 * tokenised engine identifiers (`OOC_Stanton_3_Hurston_LZ_01`)
 * always have at least one underscore.
 *
 * No-op when `lookup` is empty or undefined — paired-but-pre-fetch
 * tray surfaces should pass `undefined` and get back the raw string,
 * which is still readable.
 */
export function prettifySummary(
  raw: string,
  lookup: PrettyLookup | undefined,
): string {
  if (!lookup || lookup.size === 0 || !raw) return raw;
  // Class-name tokens are uppercase-led ASCII with at least one
  // underscore. The trailing segment is allowed lowercase so
  // names like `AEGS_Avenger_Stalker` (where `Avenger`/`Stalker`
  // are mixed-case) match cleanly.
  return raw.replace(/[A-Z][A-Z0-9]*_[A-Za-z0-9_]+/g, (match) => {
    return lookup.get(match.toLowerCase()) ?? match;
  });
}

/**
 * Sentence-case a snake_case event_type for display. Last-resort
 * fallback so an unmapped variant still reads tolerably ("player
 * incapacitated" rather than "player_incapacitated").
 */
export function humanizeEventType(type: string): string {
  if (!type) return 'Event';
  const words = type.split('_');
  return words
    .map((w, i) => (i === 0 ? (w[0] ?? '').toUpperCase() + w.slice(1) : w))
    .join(' ');
}

/**
 * Pick the best player-facing title for a TimelineEntry-shaped row.
 * Prefers the server-formatted `summary` (which is the
 * `format_summary(&GameEvent)` output Rust-side, e.g. "PlayerDeath:
 * zone=Stanton") when it's non-trivial. Falls back to the verb table,
 * then to a sentence-cased event_type. Never returns the raw
 * snake_case key — that's the whole point of this helper.
 */
export function humanTitleForEntry(
  entry: {
    event_type: string;
    summary: string;
  },
  lookup?: PrettyLookup,
): string {
  const trimmed = entry.summary.trim();
  // The Rust-side fallback for unparseable payloads is
  // `"{event_type} (unparseable payload)"`. Detect and route around
  // it so we don't surface the raw snake_case key as the headline.
  const isFallback = trimmed.startsWith(`${entry.event_type} (unparseable`);
  if (trimmed && !isFallback) {
    return prettifySummary(trimmed, lookup);
  }
  return EVENT_VERB_TABLE[entry.event_type] ?? humanizeEventType(entry.event_type);
}

/**
 * Pick the best player-facing title for an EventEnvelope (the
 * api-client-ts wire shape — has full `metadata.primary_entity`
 * access). Prefers `metadata.primary_entity.display_name` when the
 * server stamped one, so a death row reads "Lt. Joe" rather than
 * "player_death". Falls through to the same verb table /
 * sentence-case fallback as `humanTitleForEntry` for events without
 * a primary_entity (or pre-metadata legacy rows).
 *
 * Note: we deliberately don't fold this into `humanTitleForEntry`.
 * The `TimelineEntry` Tauri command projection doesn't carry
 * metadata yet — surfacing it on that surface is a separate piece
 * of work — so the two helpers stay distinct and each takes the
 * shape its callers actually have.
 */
export function humanTitleForEnvelope(
  env: EventEnvelope,
  lookup?: PrettyLookup,
): string {
  const display = env.metadata?.primary_entity?.display_name;
  if (display && display.trim()) {
    // primary_entity.display_name is already a friendly name from
    // the server's reference resolution — no catalog lookup needed,
    // but run it through the prettifier as a safety net for cases
    // where the server hasn't backfilled metadata yet.
    return prettifySummary(display.trim(), lookup);
  }
  const type = env.event?.type ?? '';
  return EVENT_VERB_TABLE[type] ?? humanizeEventType(type);
}
