import type { ShareScope } from '@/lib/api';

/**
 * State, in plain language, what a stranger gets from a public profile.
 *
 * The toggle used to say "anyone can view your summary and timeline",
 * which is true of almost any setting and therefore tells the owner
 * nothing. Meanwhile the public path had no scope at all, so the honest
 * version of that sentence was "every event type you have ever logged,
 * with counts, plus a 90-day heatmap" — which nobody would have guessed
 * from the copy.
 *
 * Derived from the scope the server actually stores, never from what
 * the form is about to send: a page that describes the intent rather
 * than the stored state will happily narrate a write that failed.
 *
 * `null` scope is the pre-scope state — a profile made public before
 * the clamp existed. It is genuinely uncapped, and says so, because the
 * owner is the one who should decide whether that is what they wanted.
 */
export interface PublicScopeDescription {
  /** One line per thing a stranger can read. */
  published: string[];
  /** True when nothing is clamping the profile at all. */
  uncapped: boolean;
}

/** Longest window the public timeline will serve without a clamp. */
const MAX_PUBLIC_WINDOW_DAYS = 90;

export function describePublicScope(
  scope: ShareScope | null | undefined,
): PublicScopeDescription {
  const maxTypes = scope?.max_event_types ?? null;
  const windowDays = scope?.window_days ?? MAX_PUBLIC_WINDOW_DAYS;
  // "Uncapped" means the two clamps that actually bound the payload are
  // both absent. A scope carrying only, say, a widget list still leaves
  // the histogram wide open, so it does not count as capped here.
  const uncapped = maxTypes === null && scope?.window_days == null;

  const published = [
    'Your total number of logged events',
    maxTypes === null
      ? 'Every event type you have logged, with counts'
      : `Your ${maxTypes} busiest event types, with counts`,
    `An activity heatmap covering the last ${windowDays} days`,
    'Your RSI profile card, enlistment date and orgs',
  ];

  return { published, uncapped };
}

/**
 * The one-line version, for a chip or a status row.
 *
 * Deliberately leads with the histogram: it is the field that surprised
 * people, and the one the default now clamps.
 */
export function summarisePublicScope(
  scope: ShareScope | null | undefined,
): string {
  const maxTypes = scope?.max_event_types ?? null;
  const windowDays = scope?.window_days ?? MAX_PUBLIC_WINDOW_DAYS;
  return maxTypes === null
    ? `All event types · ${windowDays} days`
    : `Top ${maxTypes} event types · ${windowDays} days`;
}
