/**
 * Submission status pill. Maps the API's lowercase status string to a
 * human-readable label + the appropriate `ss-badge` colour variant.
 *
 * Lives outside the route folder so both `/submissions` and
 * `/submissions/[id]` can import it without forcing the list page to
 * re-export internals (Next.js bans non-default exports from `page.tsx`).
 */

export function StatusPill({ status }: { status: string }) {
  const map: Record<string, { kind: string; label: string }> = {
    review: { kind: 'ss-badge--warn', label: 'Under review' },
    accepted: { kind: 'ss-badge--ok', label: 'Accepted' },
    shipped: { kind: 'ss-badge--ok', label: 'Shipped' },
    rejected: { kind: 'ss-badge--danger', label: 'Rejected' },
    flagged: { kind: 'ss-badge--danger', label: 'Flagged' },
    withdrawn: { kind: '', label: 'Withdrawn' },
  };
  const m = map[status] ?? { kind: '', label: status };
  return <span className={`ss-badge ${m.kind}`.trim()}>{m.label}</span>;
}
