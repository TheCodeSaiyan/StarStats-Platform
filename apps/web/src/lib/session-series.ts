/**
 * Pure helpers turning the (newest-first) sessions list into a small
 * numeric series suitable for an inline sparkline.
 *
 * The chosen metric is per-session PLAYTIME (duration in minutes) —
 * the most reliable per-session number the API exposes: every closed
 * session carries `started_at` + `ended_at`, so the duration needs no
 * extra endpoint. Event-count is noisier (a quiet mining run and a
 * frantic dogfight both count "events"), so duration reads truer as a
 * "how long did I play" trend.
 */

/** Minimal shape needed to derive a session duration. */
export interface SessionDurationInput {
  started_at?: string | null;
  ended_at?: string | null;
}

/** Parse a session's duration in whole minutes, or null when it can't
 *  be computed (open session, missing/invalid timestamps, non-positive
 *  span). Kept separate so the filtering rule is testable in isolation. */
function sessionDurationMinutes(s: SessionDurationInput): number | null {
  if (!s.started_at || !s.ended_at) return null;
  const a = new Date(s.started_at).getTime();
  const b = new Date(s.ended_at).getTime();
  if (Number.isNaN(a) || Number.isNaN(b) || b <= a) return null;
  return Math.round((b - a) / 60_000);
}

/**
 * Last-N-sessions playtime series (minutes), OLDEST-FIRST.
 *
 * The input list is newest-first (list[0] = most recent), matching the
 * `/v1/users/:handle/sessions` contract. We take the most recent `n`
 * sessions that have a computable duration, then reverse so the series
 * reads left -> right through time (a sparkline convention). Open
 * sessions and unparseable rows are skipped rather than zero-filled so
 * the trend reflects real completed play, not gaps.
 *
 * Returns fewer than `n` entries when the user has fewer qualifying
 * sessions, and an empty array when none qualify — callers should treat
 * a <2-length result as "not enough to plot a trend".
 */
export function lastNSessionDurationsMinutes(
  sessions: ReadonlyArray<SessionDurationInput>,
  n = 5,
): number[] {
  const durations: number[] = [];
  for (const s of sessions) {
    const mins = sessionDurationMinutes(s);
    if (mins !== null) durations.push(mins);
    if (durations.length >= n) break;
  }
  // `durations` is newest-first (we walked the newest-first list);
  // reverse to oldest-first for left-to-right reading.
  return durations.reverse();
}
