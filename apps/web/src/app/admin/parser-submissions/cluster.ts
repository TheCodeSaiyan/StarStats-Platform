/**
 * Groups moderator-queue rows by their `coarse_shape` so a rule author can
 * triage a family of near-duplicate unknown-line shapes as one unit instead
 * of scrolling past every individual `shape_hash`.
 *
 * NOTE: this groups only the page of rows the caller already fetched (the
 * pending set is small today). It is not a substitute for a server-side
 * `/clusters` endpoint should the backlog grow large enough that clustering
 * needs to span pages — that's the future growth path.
 */

import type { AdminParserSubmissionSummary } from '@/lib/api';

export type Cluster = {
  coarseShape: string;
  members: AdminParserSubmissionSummary[];
  totalOccurrences: number;
  totalSubmitters: number;
  representative: AdminParserSubmissionSummary; // highest total_occurrence_count
};

export function clusterSubmissions(
  rows: AdminParserSubmissionSummary[],
): Cluster[] {
  const byKey = new Map<string, AdminParserSubmissionSummary[]>();
  for (const r of rows) {
    const k = r.coarse_shape;
    (byKey.get(k) ?? byKey.set(k, []).get(k)!).push(r);
  }
  const clusters: Cluster[] = [...byKey.entries()].map(
    ([coarseShape, members]) => ({
      coarseShape,
      members,
      totalOccurrences: members.reduce(
        (s, m) => s + m.total_occurrence_count,
        0,
      ),
      totalSubmitters: members.reduce((s, m) => s + m.submitter_count, 0),
      representative: members.reduce((a, b) =>
        b.total_occurrence_count > a.total_occurrence_count ? b : a,
      ),
    }),
  );
  // Rank by summed impact desc; stable tiebreak by coarseShape.
  clusters.sort(
    (a, b) =>
      b.totalOccurrences - a.totalOccurrences ||
      a.coarseShape.localeCompare(b.coarseShape),
  );
  return clusters;
}
