/**
 * Pure builder for the Sessions widget's summary line
 * ("N sessions · Xh played").
 *
 * Why this exists: the session LIST endpoint caps at
 * `SESSIONS_LIST_LIMIT` (50) newest-first sessions, so deriving the
 * totals from that list silently undercounts heavy users (and the
 * count pins at exactly the cap). When the viewer is the profile
 * owner we instead pass the true lifetime aggregate from
 * `/v1/me/stats/playtime?all_time=true`. Visitors (no me-scoped
 * lifetime endpoint) fall back to the capped list, labelled honestly
 * as `N+` when the cap was hit so the number doesn't read as exact.
 */
export interface SessionSummaryLine {
  /** e.g. "128 sessions", "1 session", or "50+ sessions" (capped). */
  countLabel: string;
  /** e.g. "150h played", or null when total playtime is zero/unknown. */
  totalHoursLabel: string | null;
}

export interface SessionSummaryInput {
  /** Lifetime aggregate (owner only); null for visitors / fetch failure. */
  lifetime: { session_count: number; total_playtime_secs: number } | null;
  /** Number of sessions returned by the (capped) list endpoint. */
  listLength: number;
  /** Summed duration of the returned sessions, in ms (fallback path). */
  derivedTotalMs: number;
  /** Server-side cap on the list (mirrors SESSIONS_LIST_LIMIT). */
  listCap: number;
}

export function buildSessionSummary({
  lifetime,
  listLength,
  derivedTotalMs,
  listCap,
}: SessionSummaryInput): SessionSummaryLine {
  const count = lifetime ? lifetime.session_count : listLength;
  // Only the fallback (list-derived) path can be truncated; the
  // lifetime aggregate is exact.
  const capped = !lifetime && listLength >= listCap;
  const noun = count === 1 ? 'session' : 'sessions';
  const countLabel = `${count.toLocaleString()}${capped ? '+' : ''} ${noun}`;

  const totalMs = lifetime ? lifetime.total_playtime_secs * 1000 : derivedTotalMs;
  const totalHoursLabel =
    totalMs > 0 ? `${Math.round(totalMs / 3_600_000)}h played` : null;

  return { countLabel, totalHoursLabel };
}
