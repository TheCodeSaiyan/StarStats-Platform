/**
 * Canonical time-range selector model. Extracted from the journey
 * RangeBar so widgets + the profile/Me pages can share it without
 * importing from a journey component (journey is removed in Plan 4).
 *
 * `id` is the URL token; `hours` feeds endpoints that take an `hours`
 * window; `label` is what the user sees.
 */
import type { MetricsRange } from '@/lib/api';

export const RANGES = [
  { id: '24h', label: '24h', hours: 24 },
  { id: '7d', label: '7d', hours: 24 * 7 },
  { id: '30d', label: '30d', hours: 24 * 30 },
  { id: '90d', label: '90d', hours: 24 * 90 },
  { id: 'all', label: 'All', hours: 24 * 365 },
] as const;

export type RangeId = (typeof RANGES)[number]['id'];

const RANGE_IDS = RANGES.map((r) => r.id) as readonly string[];

/** Default window for /me + profile when no `?range=` is set. 7 days is
 *  the "what have I been up to lately" default — recent enough to be
 *  meaningful, wide enough to have data. */
export const DEFAULT_RANGE: RangeId = '7d';

/** Parse a `?range=` value into a known id, defaulting to {@link DEFAULT_RANGE}. */
export function parseRange(raw: string | undefined): RangeId {
  if (raw && RANGE_IDS.includes(raw)) return raw as RangeId;
  return DEFAULT_RANGE;
}

export function rangeToHours(id: RangeId): number {
  return RANGES.find((r) => r.id === id)!.hours;
}

/** Hours to send as an `hours=` window for a given bucket.
 *
 *  `all` is 8760 hours — 365 days — and that is CORRECT, not a bug:
 *  365 days is the hard retention limit, so "everything we have" and
 *  "the last year" are the same set. An earlier version sent `undefined`
 *  here to mean lifetime; that promised a depth the data does not have,
 *  and diverged from the server, which now bounds `all` to 365 too.
 *
 *  Kept as a named helper rather than inlining `rangeToHours` so the
 *  retention assumption has one place to change when the limit moves. */
export function rangeToWindowHours(id: RangeId): number {
  return rangeToHours(id);
}

/** Whether a lifetime baseline actually contextualises this range.
 *
 *  False for `all` only. `all` spans 365 days, which is the whole of
 *  retention, so its lifetime twin covers the same rows — the widget
 *  would render "12,282 of 12,282", restating the figure instead of
 *  comparing it. A comparison that says nothing is worse than none:
 *  it occupies the space where real context belongs.
 *
 *  Deliberately keyed on the range, not on whether the two values
 *  happen to be equal. On a narrower range a coincidental match is
 *  real information ("everything you have done was in the last 30
 *  days"); on `all` the equality is true by construction and can
 *  never mean anything. */
export function rangeHasLifetimeBaseline(id: RangeId): boolean {
  return id !== 'all';
}

/** Day-count window for endpoints that take `days` (e.g. timeline). */
export function rangeToDays(id: RangeId): number {
  switch (id) {
    case '24h':
      return 1;
    case '7d':
      return 7;
    case '30d':
      return 30;
    case '90d':
      return 90;
    case 'all':
      return 365;
  }
}

/** Map to the `MetricsRange` accepted by /v1/me/metrics/event-types.
 *
 *  Identity: every bucket the selector offers is now a bucket the
 *  endpoint serves. This used to widen '24h' to '7d' because the server
 *  had no 24h option, so picking "24h" rendered a WEEK under a "24h"
 *  label — a confidently wrong number, which is worse than a missing
 *  one. The server gained the bucket rather than the client hiding the
 *  gap. */
export function rangeToMetricsRange(id: RangeId): MetricsRange {
  return id;
}

/** ISO-8601 lower bound for `?since=` filters: now minus the range's
 *  hour window. Used by event-stream widgets (recent_activity, loadout). */
export function rangeToSinceIso(id: RangeId): string {
  return new Date(Date.now() - rangeToHours(id) * 3600_000).toISOString();
}

/** Human-readable label, e.g. "last 7 days". */
export function rangeLabel(id: RangeId): string {
  switch (id) {
    case '24h':
      return 'last 24 hours';
    case '7d':
      return 'last 7 days';
    case '30d':
      return 'last 30 days';
    case '90d':
      return 'last 90 days';
    case 'all':
      // Not "all time" — 365 days is the retention limit, so this is
      // genuinely everything retained. The label says what it is.
      return 'last year';
  }
}
