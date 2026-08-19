/**
 * Shared widget formatters. Every widget used to inline its own duration /
 * relative-time / number helpers (the audit found the same `fmtDuration`
 * duplicated across sessions, records, lives). One copy here.
 */

/** `93784` → `"1d 2h"`, `5400` → `"1h 30m"`, `90` → `"1m"`, `0` → `"0m"`.
 *  Two most-significant units, never more — dense HUD readouts. */
export function fmtDuration(secs: number): string {
  const s = Math.max(0, Math.round(secs));
  const d = Math.floor(s / 86400);
  const h = Math.floor((s % 86400) / 3600);
  const m = Math.floor((s % 3600) / 60);
  if (d > 0) return h > 0 ? `${d}d ${h}h` : `${d}d`;
  if (h > 0) return m > 0 ? `${h}h ${m}m` : `${h}h`;
  return `${m}m`;
}

/** Whole hours as `"418h"`. */
export function fmtHours(hours: number): string {
  return `${Math.round(hours).toLocaleString()}h`;
}

/** Compact integer with thousands separators. */
export function fmtNum(n: number): string {
  return Math.round(n).toLocaleString();
}

/** A percentage `0.69` → `"69%"` (fraction in) or `69` → `"69%"` (already a
 *  percent when `alreadyPercent`). */
export function fmtPct(value: number, alreadyPercent = false): string {
  const pct = alreadyPercent ? value : value * 100;
  return `${Math.round(pct)}%`;
}

/** `"2026-07-14T…"` → `"3d ago"` / `"5h ago"` / `"just now"`. `now` is
 *  injected (server render has no stable clock; callers pass Date.now()). */
export function fmtRelative(iso: string | null | undefined, now: number): string {
  if (!iso) return '';
  const then = Date.parse(iso);
  if (Number.isNaN(then)) return '';
  const secs = Math.max(0, Math.round((now - then) / 1000));
  if (secs < 60) return 'just now';
  const mins = Math.floor(secs / 60);
  if (mins < 60) return `${mins}m ago`;
  const hrs = Math.floor(mins / 60);
  if (hrs < 24) return `${hrs}h ago`;
  const days = Math.floor(hrs / 24);
  return `${days}d ago`;
}

/** Sum selected keys out of an event-type breakdown `{event_type,count}[]`.
 *  Duplicated in travel + combat_mission before this. */
export function sumCounts(
  types: ReadonlyArray<{ event_type: string; count: number }> | null | undefined,
  keys: readonly string[],
): number {
  if (!types) return 0;
  const wanted = new Set(keys);
  return types.reduce((acc, t) => (wanted.has(t.event_type) ? acc + t.count : acc), 0);
}

/** Turn an event-type breakdown into a `{key: count}` map. */
export function countsByType(
  types: ReadonlyArray<{ event_type: string; count: number }> | null | undefined,
): Record<string, number> {
  const out: Record<string, number> = {};
  for (const t of types ?? []) out[t.event_type] = t.count;
  return out;
}
