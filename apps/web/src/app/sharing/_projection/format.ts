/**
 * Formatting helpers for the sharing surface.
 *
 * Lifted VERBATIM out of `page.tsx` when the projection port split the page
 * into section components — they were page-local and two of them are now
 * needed by `InboundList` and `ProfileViewsPane` as well. Behaviour is
 * unchanged; this is a move, not a rewrite.
 */

/** Format a timestamp as a short relative string like "3d ago" or
 *  "just now". Returns null for missing input so the caller can
 *  conditionally render. */
export function formatRelativePast(
  iso: string | null | undefined,
): string | null {
  if (!iso) return null;
  const ts = new Date(iso);
  if (Number.isNaN(ts.getTime())) return null;
  const diffMs = Date.now() - ts.getTime();
  if (diffMs < 0) return 'just now';
  const min = Math.round(diffMs / 60_000);
  if (min < 1) return 'just now';
  if (min < 60) return `${min}m ago`;
  const hr = Math.round(min / 60);
  if (hr < 24) return `${hr}h ago`;
  const day = Math.round(hr / 24);
  return `${day}d ago`;
}

/** Format an ISO timestamp as "in 3d" / "expired" / "in 2h" for the
 *  share pills. Returns null when no expiry was set. */
export function formatExpiry(iso: string | null | undefined): string | null {
  if (!iso) return null;
  const ts = new Date(iso);
  if (Number.isNaN(ts.getTime())) return null;
  const now = Date.now();
  const diffMs = ts.getTime() - now;
  if (diffMs <= 0) return 'expired';
  const diffMin = Math.round(diffMs / 60_000);
  if (diffMin < 60) return `in ${diffMin}m`;
  const diffHr = Math.round(diffMin / 60);
  if (diffHr < 24) return `in ${diffHr}h`;
  const diffDay = Math.round(diffHr / 24);
  return `in ${diffDay}d`;
}

/** Per-source view breakdown as one sentence. Sources with no views are
 *  omitted rather than shown as zeros. */
export function renderBreakdown(bySource: Record<string, number>): string {
  const labels: Array<[string, string]> = [
    ['direct', 'from direct links'],
    ['discover', 'from discover'],
    ['shared', 'from shared profiles'],
    ['other', 'from other'],
  ];
  const parts: string[] = [];
  for (const [key, label] of labels) {
    const n = bySource[key] ?? 0;
    if (n > 0) parts.push(`${n} ${label}`);
  }
  return parts.length === 0 ? 'No source data yet.' : parts.join(' · ');
}
