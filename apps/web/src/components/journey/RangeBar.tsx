/**
 * URL-synced time-range chip selector for journey stats tabs.
 *
 * The server-side stats endpoints all accept an `hours` parameter
 * (currently min 1, max 24*365 = 1y per `STATS_MAX_HOURS`). This
 * component renders a chip row of common windows and links each to
 * `/journey?view=<tab>&range=<id>`, preserving the active tab so
 * users don't lose their place when widening or narrowing the
 * timeframe.
 *
 * Server component — no client JS. URL is the source of truth.
 */

import React from 'react';
import Link from 'next/link';
import type { Route } from 'next';
import { RANGES, type RangeId } from '@/lib/range';

// Re-export the shared helpers so existing journey importers keep
// working unchanged (parseRange/rangeToHours/rangeLabel moved to
// @/lib/range in Plan 2).
export { parseRange, rangeToHours, rangeLabel } from '@/lib/range';
export type { RangeId } from '@/lib/range';

export function RangeBar({
  active,
  buildHref,
}: {
  active: RangeId;
  /** Callback that maps a range id to a Route. Callers control URL
   *  shape so the same chip strip can drive `/journey` (preserves
   *  view) and `/dashboard` (no view concept). */
  buildHref: (id: RangeId) => Route;
}) {
  return (
    <nav
      aria-label="Time range"
      style={{
        display: 'flex',
        gap: 4,
        flexWrap: 'wrap',
        alignItems: 'center',
      }}
    >
      <span
        className="ss-eyebrow"
        style={{ marginRight: 6, color: 'var(--fg-dim)' }}
      >
        Range
      </span>
      {RANGES.map((r) => {
        const isActive = r.id === active;
        return (
          <Link
            key={r.id}
            href={buildHref(r.id)}
            aria-current={isActive ? 'page' : undefined}
            className="mono"
            style={{
              padding: '4px 10px',
              fontSize: 12,
              borderRadius: 0,
              textDecoration: 'none',
              color: isActive ? 'var(--bg)' : 'var(--fg-muted)',
              background: isActive ? 'var(--accent)' : 'transparent',
              border: `1px solid ${isActive ? 'var(--accent)' : 'var(--border)'}`,
              letterSpacing: '0.02em',
            }}
          >
            {r.label}
          </Link>
        );
      })}
    </nav>
  );
}

