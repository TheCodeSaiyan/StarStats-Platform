/**
 * One-line "today vs typical" comparison. Compares the user's most-
 * visited location TODAY (24h trace) to where they USUALLY spend
 * time (7d breakdown) and surfaces a one-liner.
 *
 * Renders nothing when either window is empty or when the comparison
 * adds no signal (same top AND share within a few points). The bar
 * is a glanceable insight, not a mandatory tile — quiet is the right
 * default when there's nothing interesting to say.
 */

import React from 'react';
import type { BreakdownResponse } from '@/lib/api';
import { type DistinctStop } from './trail-utils';

interface Props {
  /** Distinct stops from the 24h trace, oldest→newest. */
  todayStops: DistinctStop[];
  /** 7d aggregate dwell. */
  typicalBreakdown: BreakdownResponse | null;
}

export function LocationTypicalInsight({
  todayStops,
  typicalBreakdown,
}: Props) {
  const today = topByEvents(todayStops);
  const typical = topByDwell(typicalBreakdown?.entries ?? []);

  if (!today || !typical) return null;

  const sameTop = sameLocation(today, typical);
  const closeShare = Math.abs(today.pct - typical.pct) < 5;
  if (sameTop && closeShare) return null;

  const sentence = sameTop
    ? `${Math.round(today.pct)}% of today's activity was in ${today.label}, vs ${Math.round(typical.pct)}% typically.`
    : `Today's activity skewed to ${today.label} (${Math.round(today.pct)}%); your typical week is anchored on ${typical.label} (${Math.round(typical.pct)}%).`;

  return (
    <aside
      role="note"
      style={{
        display: 'flex',
        alignItems: 'flex-start',
        gap: 10,
        padding: '10px 14px',
        background: 'var(--bg-elev)',
        border: '1px solid var(--border)',
        borderLeft: '3px solid var(--accent)',
        borderRadius: 0,
        fontSize: 13,
        color: 'var(--fg)',
        lineHeight: 1.55,
      }}
    >
      <span aria-hidden style={{ color: 'var(--accent)', lineHeight: 1.55 }}>
        ✦
      </span>
      <span>{sentence}</span>
    </aside>
  );
}

interface TopSlice {
  label: string;
  system: string | null;
  planet: string | null;
  city: string | null;
  /** Share of the window this location accounts for (0–100). */
  pct: number;
}

function topByEvents(stops: DistinctStop[]): TopSlice | null {
  if (stops.length === 0) return null;
  const byKey = new Map<string, { count: number; sample: DistinctStop }>();
  let total = 0;
  for (const s of stops) {
    total += s.eventCount;
    const cur = byKey.get(s.key);
    if (cur) {
      cur.count += s.eventCount;
    } else {
      byKey.set(s.key, { count: s.eventCount, sample: s });
    }
  }
  if (total === 0) return null;
  const top = [...byKey.values()].sort((a, b) => b.count - a.count)[0];
  return {
    // Prefer the server-classified friendly name so the insight prose
    // never surfaces a raw engine identifier.
    label: top.sample.resolvedLabel ?? top.sample.label,
    system: top.sample.system,
    planet: top.sample.planet,
    city: top.sample.city,
    pct: (top.count / total) * 100,
  };
}

function topByDwell(
  entries: BreakdownResponse['entries'],
): TopSlice | null {
  if (entries.length === 0) return null;
  const total = entries.reduce((s, e) => s + e.dwell_seconds, 0);
  if (total === 0) return null;
  const sorted = [...entries].sort(
    (a, b) => b.dwell_seconds - a.dwell_seconds,
  );
  const top = sorted[0];
  const label = top.city ?? top.planet ?? top.system ?? 'Unknown';
  return {
    label,
    system: top.system ?? null,
    planet: top.planet ?? null,
    city: top.city ?? null,
    pct: (top.dwell_seconds / total) * 100,
  };
}

function sameLocation(a: TopSlice, b: TopSlice): boolean {
  return (
    (a.city ?? '') === (b.city ?? '') &&
    (a.planet ?? '') === (b.planet ?? '') &&
    (a.system ?? '') === (b.system ?? '')
  );
}
