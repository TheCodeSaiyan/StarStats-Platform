/**
 * Freshness classification for the parser-health detector's last run.
 *
 * Lives outside `page.tsx` because Next's App Router only permits a fixed set
 * of exports from a page module (`default`, `metadata`, `revalidate`, …); an
 * extra named export fails the production type check even though `tsc
 * --noEmit`, eslint and vitest all pass. Keeping it here also lets the logic
 * be tested directly, which matters more than usual: this function decides
 * whether "no findings" reads as reassuring or meaningless, and getting it
 * wrong reintroduces exactly the blind spot the feature removes.
 */
import type { ParserHealthRun } from '@/lib/api';

/** A run older than this means the daily loop has stopped. */
export const RUN_STALE_AFTER_HOURS = 36;

export type RunState = 'never' | 'stale' | 'failed' | 'ok';

export function runStaleness(
  run: ParserHealthRun | null | undefined,
  now: number,
): { state: RunState; ageHours: number | null } {
  if (!run) return { state: 'never', ageHours: null };
  // Fall back to started_at for a pass that began and never finished — a
  // crashed mid-pass must not be measured as infinitely old, nor as healthy.
  const stamp = run.finished_at ?? run.started_at;
  const ageHours = (now - new Date(stamp).getTime()) / 3_600_000;
  if (run.error) return { state: 'failed', ageHours };
  if (!run.finished_at || ageHours > RUN_STALE_AFTER_HOURS) {
    return { state: 'stale', ageHours };
  }
  return { state: 'ok', ageHours };
}
