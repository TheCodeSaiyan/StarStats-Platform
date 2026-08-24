/**
 * Admin · Contract catalog gap diagnostic.
 *
 * `contract_runs` records every contract players actually ran;
 * `contracts` holds what the catalog has published. Only 48.8% of
 * run occurrences match a catalog name exactly (see Task 1). This
 * page lists the run-observed names with no catalog match, ranked by
 * `run_count` DESC — occurrence, not distinct name count — so a
 * maintainer can see (and prioritise publishing) the biggest gaps at
 * a glance. Combat Gauntlet is ~5% of distinct unmatched names but
 * 37% of all runs; a name-sorted list would bury that entirely, so
 * the rows below are rendered in the exact order the server returns
 * them. Do NOT re-sort by name, date, or anything else.
 *
 * Auth: gated `RequireModerator` server-side (read-only diagnostic,
 * same posture as `admin_routes::list_audit`) — the parent layout
 * also enforces moderator/admin role for the whole /admin subtree.
 * This page still calls `getSession()` for type narrowing and a
 * defensive redirect, mirroring `admin/audit/page.tsx`.
 */

// Explicit React import: this repo's vitest uses the classic JSX
// runtime, so a JSX-rendering component 500s with "React is not
// defined" under test without it (the prod Next build uses the
// automatic runtime and doesn't need it).
import React from 'react';
import { Plane } from 'holo';
import { redirect } from 'next/navigation';
import {
  ApiCallError,
  getAdminContractGaps,
  type ContractGapDto,
} from '@/lib/api';
import { getSession } from '@/lib/session';
import { AdminPageHeader } from '../_components/AdminPageHeader';

export default async function ContractGapsPage() {
  const session = await getSession();
  if (!session) redirect('/auth/login?next=/admin/contract-gaps');

  let gaps: ContractGapDto[];
  let totalUnmatchedRuns: number;
  try {
    ({ gaps, total_unmatched_runs: totalUnmatchedRuns } =
      await getAdminContractGaps(session.token));
  } catch (e) {
    if (e instanceof ApiCallError && e.status === 401) {
      redirect('/auth/login?next=/admin/contract-gaps');
    }
    if (e instanceof ApiCallError && e.status === 403) redirect('/me');
    throw e;
  }

  return (
    <div
      style={{ display: 'flex', flexDirection: 'column', gap: 20 }}
    >

      <AdminPageHeader
        eyebrow="Admin · contract catalog"
        title="Catalog gaps"
        lede={
          <>
            {totalUnmatchedRuns.toLocaleString()} run occurrences have no
            matching row in the published catalog. Rows below are ranked by
            how many runs each missing name accounts for — publishing the
            names at the top closes the biggest gaps first.
          </>
        }
      />

      <Plane tilt="flat">
        {gaps.length === 0 ? (
          <p
            style={{
              margin: 0,
              padding: '40px 24px',
              textAlign: 'center',
              color: 'var(--fg-muted)',
              fontSize: 14,
            }}
          >
            No catalog gaps — every observed run matches a published name.
          </p>
        ) : (
          <table
            style={{
              width: '100%',
              borderCollapse: 'collapse',
              fontSize: 13,
              tableLayout: 'fixed',
            }}
          >
            <thead>
              <tr style={{ background: 'var(--bg-elev)' }}>
                <Th>Name</Th>
                <Th width="100px">Runs</Th>
                <Th width="140px">Distinct handles</Th>
                <Th width="180px">First seen</Th>
                <Th width="180px">Last seen</Th>
              </tr>
            </thead>
            <tbody>
              {gaps.map((gap) => (
                <GapRow key={gap.name} gap={gap} />
              ))}
            </tbody>
          </table>
        )}
      </Plane>
    </div>
  );
}

function Th({
  children,
  width,
}: {
  children: React.ReactNode;
  width?: string;
}) {
  return (
    <th
      style={{
        textAlign: 'left',
        padding: '10px 14px',
        fontWeight: 600,
        color: 'var(--fg-muted)',
        fontSize: 11,
        letterSpacing: '0.06em',
        textTransform: 'uppercase',
        borderBottom: '1px solid var(--border)',
        width,
      }}
    >
      {children}
    </th>
  );
}

function Td({ children }: { children: React.ReactNode }) {
  return (
    <td style={{ padding: '10px 14px', verticalAlign: 'top' }}>{children}</td>
  );
}

function GapRow({ gap }: { gap: ContractGapDto }) {
  return (
    <tr style={{ borderBottom: '1px solid var(--border)' }}>
      <Td>{gap.name}</Td>
      <Td>
        <span className="mono">{gap.run_count.toLocaleString()}</span>
      </Td>
      <Td>
        <span className="mono">{gap.distinct_handles.toLocaleString()}</span>
      </Td>
      <Td>
        <span className="mono" style={{ fontSize: 12 }}>
          {formatTimestamp(gap.first_seen)}
        </span>
      </Td>
      <Td>
        <span className="mono" style={{ fontSize: 12 }}>
          {formatTimestamp(gap.last_seen)}
        </span>
      </Td>
    </tr>
  );
}

/** `first_seen`/`last_seen` are nullable on the wire (a gap name can
 *  theoretically have zero surviving runs after supersede exclusion).
 *  Render a dash rather than feeding `null` to `Date` and printing
 *  "Invalid Date". */
function formatTimestamp(iso: string | null | undefined): string {
  if (!iso) return '—';
  return iso.replace('T', ' ').replace(/(\.\d+)?Z$/, '').slice(0, 19);
}
