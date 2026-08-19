/**
 * Shared presentation for the admin user-management fields.
 *
 * The two rules encoded here both exist because getting them wrong
 * FAILS SILENTLY — the page renders, and the wrong answer looks real:
 *
 *   * `last_activity_at` is nullable. A user who has never sent an
 *     event has no timestamp, and coercing that null into a Date gives
 *     "just now" (from `new Date()`) or "56 years ago" (from the
 *     epoch). Null renders the word "never".
 *
 *   * `retention_days` is nullable and null means UNLIMITED, not zero.
 *     Rendering it as "0 days" would tell an operator that a
 *     supporter's data is purged immediately.
 */

// Explicit React import: this repo's vitest uses the classic JSX
// runtime, so a JSX-rendering component ReferenceErrors without it.
import React from 'react';

export type SyncState = 'never' | 'off' | 'stale' | 'live';

const SYNC_LABELS: Record<SyncState, { label: string; hint: string }> = {
  live: { label: 'Live', hint: 'Sync on, seen within the last 7 days' },
  stale: { label: 'Stale', hint: 'Sync on, but nothing received for over 7 days' },
  off: { label: 'Off', hint: 'Paired devices exist, but none have sync enabled' },
  never: { label: 'Never', hint: 'No devices have ever been paired' },
};

/**
 * Four distinct states, not a boolean: "off" is a user choice, "stale"
 * is usually a broken install, and "never" means pairing never
 * completed. Collapsing them loses the operator's next action.
 */
export function SyncChip({ state }: { state: string }) {
  const known = (Object.keys(SYNC_LABELS) as SyncState[]).includes(
    state as SyncState,
  )
    ? (state as SyncState)
    : 'never';
  const { label, hint } = SYNC_LABELS[known];

  const color =
    known === 'live'
      ? 'var(--ok, var(--accent))'
      : known === 'stale'
        ? 'var(--warn, var(--fg-muted))'
        : 'var(--fg-dim)';

  return (
    <span
      className="ss-badge"
      title={hint}
      style={{ fontSize: 10, color, borderColor: color }}
    >
      {label}
    </span>
  );
}

/** Coarse relative time. `null` is "never" — never a date. */
export function relativeTime(
  iso: string | null | undefined,
  now: Date = new Date(),
): string {
  if (!iso) return 'never';
  const then = new Date(iso);
  if (Number.isNaN(then.getTime())) return 'unknown';

  const seconds = Math.round((now.getTime() - then.getTime()) / 1000);
  if (seconds < 60) return 'just now';
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.round(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.round(hours / 24);
  if (days < 365) return `${days}d ago`;
  return `${Math.round(days / 365)}y ago`;
}

/**
 * A retention window. `null` days means unlimited retention — the
 * supporter tier — and must never render as a number.
 */
export function retentionWindow(days: number | null | undefined): string {
  return days == null ? 'unlimited' : `${days} days`;
}
