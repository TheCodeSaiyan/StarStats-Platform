/**
 * Pure helpers shared by the journey-visualization components.
 *
 * The server's `TraceEntry[]` is a sequence of (location, event-count,
 * started_at) tuples — one per location-bearing event group. Visualizations
 * need richer derivations: distinct-stop chains, dwell time per stop, and
 * stable identity keys for de-duplication / link drawing.
 */

import type { TraceEntry } from '@/lib/api';

/** Most-precise label for a trace entry — what to show in a pill. */
export function primaryLabel(e: TraceEntry): string {
  return e.city ?? e.planet ?? e.system ?? 'In transit';
}

/** Secondary label for context. Returns null when nothing useful adds. */
export function secondaryLabel(e: TraceEntry): string | null {
  if (e.city && e.planet) return e.planet;
  if (e.planet && e.system && e.planet !== e.system) return e.system;
  if (e.system && !e.planet) return e.system;
  return null;
}

/**
 * Stable identity key for a trace entry. Same city/planet/system tuple
 * collapses to the same key so consecutive entries at one stop dedupe.
 */
export function locationKey(e: TraceEntry): string {
  return `${e.system ?? ''}|${e.planet ?? ''}|${e.city ?? ''}`;
}

/** A distinct stop — collapsed from N consecutive same-location entries. */
export interface DistinctStop {
  key: string;
  label: string;
  sublabel: string | null;
  system: string | null;
  planet: string | null;
  city: string | null;
  /**
   * Server-classified friendly name for this stop's raw engine key
   * (`TraceEntry.resolved_location.display_name`), when the classifier
   * produced one. Prefer this over `label` for display — `label` may be
   * a raw engine identifier (e.g. `Outpost_col_m_frm_indy_001`) that the
   * resolver couldn't map.
   */
  resolvedLabel: string | null;
  /**
   * Catalog slug from the classifier (`resolved_location.slug`), present
   * only on a confident hit. Drives the `/kb/location/{slug}` link.
   */
  resolvedSlug: string | null;
  /** Earliest started_at in the run (ISO). */
  enteredAt: string;
  /** Latest started_at in the run (ISO). Useful for dwell rendering. */
  lastSeenAt: string;
  /** Sum of event_count across the consecutive entries in this run. */
  eventCount: number;
}

/**
 * Collapse consecutive entries at the same location into distinct stops.
 * Server delivers oldest-first; we preserve that order. Callers pick the
 * tail with `.slice(-N)` for "most recent N" semantics.
 */
export function toDistinctStops(entries: TraceEntry[]): DistinctStop[] {
  const stops: DistinctStop[] = [];
  for (const e of entries) {
    const key = locationKey(e);
    const tail = stops[stops.length - 1];
    if (tail && tail.key === key) {
      tail.lastSeenAt = e.started_at;
      tail.eventCount += e.event_count;
      continue;
    }
    stops.push({
      key,
      label: primaryLabel(e),
      sublabel: secondaryLabel(e),
      system: e.system ?? null,
      planet: e.planet ?? null,
      city: e.city ?? null,
      resolvedLabel: e.resolved_location?.display_name ?? null,
      resolvedSlug: e.resolved_location?.slug ?? null,
      enteredAt: e.started_at,
      lastSeenAt: e.started_at,
      eventCount: e.event_count,
    });
  }
  return stops;
}

/**
 * Short relative time like "3m", "2h", "1d" — for hover titles and
 * compact strip labels. Returns empty string on invalid input.
 */
export function relativeAge(iso: string, now: number = Date.now()): string {
  const ts = new Date(iso).getTime();
  if (Number.isNaN(ts)) return '';
  const mins = Math.max(0, Math.floor((now - ts) / 60_000));
  if (mins < 1) return 'now';
  if (mins < 60) return `${mins}m`;
  const hrs = Math.floor(mins / 60);
  if (hrs < 24) return `${hrs}h`;
  const days = Math.floor(hrs / 24);
  return `${days}d`;
}

/**
 * Compact dwell label "3h 42m" / "47m" / "12s" — for time-spent bars
 * and timeline rows. Seconds is the wire unit (BreakdownEntry).
 */
export function formatDwell(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds < 0) return '0s';
  if (seconds < 60) return `${Math.round(seconds)}s`;
  const mins = Math.round(seconds / 60);
  if (mins < 60) return `${mins}m`;
  const hours = Math.floor(mins / 60);
  const remainderMins = mins % 60;
  return remainderMins === 0 ? `${hours}h` : `${hours}h ${remainderMins}m`;
}

/**
 * Glyph picker — keeps the visual hierarchy (city > planet > system >
 * unknown) without an icon-font dependency. Matches `LocationPill`'s
 * cosmetic choice so the strip and the pill look consistent when
 * stacked.
 */
export function glyphFor(s: Pick<DistinctStop, 'city' | 'planet'>): string {
  if (s.city) return '🛰';
  if (s.planet) return '🪐';
  return '✦';
}
