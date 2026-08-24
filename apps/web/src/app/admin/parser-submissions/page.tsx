import { Plane } from 'holo';
/**
 * Admin · Parser shapes moderation queue (W6).
 *
 * Lists tray-promoted parser_submissions rows so a rule author can
 * triage unknown-line shapes. Mirrors the admin/submissions queue
 * shape: URL-driven status filter, cursor-based pagination via the
 * server's `next_after` (single i64 token), defensive 401/403
 * redirects, AdminNav at the top.
 *
 * Distinct from /admin/submissions which is community label
 * proposals — that surface has its own moderation lifecycle. The
 * "Parser shapes" link in AdminNav points here.
 *
 * The list intentionally surfaces only the high-impact columns
 * (shape_hash, shell_tag, submitter_count, total_occurrences,
 * last_submitted_at, status) — the detail page carries the full
 * payload, raw examples, and the moderation form.
 */

import Link from 'next/link';
import type { Route } from 'next';
import { redirect } from 'next/navigation';
import {
  ApiCallError,
  getAdminParserSubmissions,
  type AdminParserSubmissionStatus,
  type AdminParserSubmissionsListResponse,
  type AdminParserSubmissionSummary,
} from '@/lib/api';
import { getSession } from '@/lib/session';
import { clusterSubmissions, type Cluster } from './cluster';
import { AdminPageHeader } from '../_components/AdminPageHeader';

interface SearchParams {
  status?: string;
  after?: string;
  view?: string;
}

const PAGE_LIMIT = 50;

type FilterId = 'pending' | 'drafting' | 'rule_written' | 'dismissed' | 'all';

type ViewId = 'flat' | 'grouped';

function parseView(raw: string | undefined): ViewId {
  return raw === 'grouped' ? 'grouped' : 'flat';
}

const FILTER_TABS: ReadonlyArray<{
  id: FilterId;
  label: string;
  /** Status param to send to the API; matches the server vocabulary. */
  apiStatus: AdminParserSubmissionStatus | 'all';
}> = [
  { id: 'pending', label: 'Pending', apiStatus: 'pending' },
  { id: 'drafting', label: 'Drafting', apiStatus: 'drafting' },
  { id: 'rule_written', label: 'Rule written', apiStatus: 'rule_written' },
  { id: 'dismissed', label: 'Dismissed', apiStatus: 'dismissed' },
  { id: 'all', label: 'All', apiStatus: 'all' },
];

function parseFilter(raw: string | undefined): FilterId {
  switch (raw) {
    case 'pending':
    case 'drafting':
    case 'rule_written':
    case 'dismissed':
    case 'all':
      return raw;
    default:
      return 'pending';
  }
}

function parseAfter(raw: string | undefined): number | undefined {
  if (!raw) return undefined;
  const n = Number.parseInt(raw, 10);
  if (!Number.isFinite(n) || n <= 0) return undefined;
  return n;
}

function relTime(rfc3339: string): string {
  const t = Date.parse(rfc3339);
  if (Number.isNaN(t)) return rfc3339;
  const seconds = Math.floor((Date.now() - t) / 1000);
  if (seconds < 60) return 'just now';
  if (seconds < 3_600) return `${Math.floor(seconds / 60)}m ago`;
  if (seconds < 86_400) return `${Math.floor(seconds / 3_600)}h ago`;
  if (seconds < 30 * 86_400) return `${Math.floor(seconds / 86_400)}d ago`;
  const days = Math.floor(seconds / 86_400);
  return `${days}d ago`;
}

export default async function AdminParserSubmissionsPage(props: {
  searchParams: Promise<SearchParams>;
}) {
  const session = await getSession();
  if (!session) redirect('/auth/login?next=/admin/parser-submissions');

  const params = await props.searchParams;
  const filter = parseFilter(params.status);
  const after = parseAfter(params.after);
  const view = parseView(params.view);
  const activeStatus = FILTER_TABS.find((t) => t.id === filter)?.apiStatus ?? 'pending';

  let listing: AdminParserSubmissionsListResponse;
  try {
    listing = await getAdminParserSubmissions(session.token, {
      status: activeStatus,
      limit: PAGE_LIMIT,
      after,
    });
  } catch (e) {
    if (e instanceof ApiCallError && e.status === 401) {
      redirect('/auth/login?next=/admin/parser-submissions');
    }
    if (e instanceof ApiCallError && e.status === 403) {
      redirect('/me');
    }
    throw e;
  }

  const buildFilterHref = (id: FilterId): Route =>
    (`/admin/parser-submissions?status=${id}&view=${view}`) as Route;

  const buildViewHref = (id: ViewId): Route =>
    (`/admin/parser-submissions?status=${filter}&view=${id}`) as Route;

  const olderHref = listing.next_after
    ? (`/admin/parser-submissions?status=${filter}&after=${listing.next_after}&view=${view}` as Route)
    : null;

  // Clustering runs over only the rows already fetched for this page (the
  // pending backlog is small today); a `/clusters` endpoint spanning the
  // full moderation queue is the growth path if that stops holding.
  const clusters: Cluster[] =
    view === 'grouped' ? clusterSubmissions(listing.submissions) : [];

  return (
    <div
      style={{ display: 'flex', flexDirection: 'column', gap: 20 }}
    >

      <AdminPageHeader
        eyebrow="Admin · parser shapes"
        title="Parser shapes"
        lede={
          <>
            Unknown-line shapes promoted from the tray. Sorted by how many
            distinct installs surfaced the shape, then by total occurrences —
            the highest-impact rules land at the top.
          </>
        }
      />

      <nav
        aria-label="Status filters"
        style={{ display: 'flex', flexWrap: 'wrap', gap: 6 }}
      >
        {FILTER_TABS.map((t) => {
          const active = t.id === filter;
          return (
            <Link
              key={t.id}
              href={buildFilterHref(t.id)}
              prefetch={false}
              data-active={active ? 'true' : undefined}
              style={{
                padding: '6px 12px',
                borderRadius: 0,
                fontSize: 13,
                textDecoration: 'none',
                border: '1px solid',
                background: active ? 'var(--bg-elev)' : 'transparent',
                borderColor: active ? 'var(--border-strong)' : 'var(--border)',
                color: active ? 'var(--fg)' : 'var(--fg-muted)',
              }}
            >
              {t.label}
            </Link>
          );
        })}
      </nav>

      <nav
        aria-label="View mode"
        style={{ display: 'flex', flexWrap: 'wrap', gap: 6 }}
      >
        {(
          [
            { id: 'flat', label: 'Flat list' },
            { id: 'grouped', label: 'Grouped by shape' },
          ] as const
        ).map((v) => {
          const active = v.id === view;
          return (
            <Link
              key={v.id}
              href={buildViewHref(v.id)}
              prefetch={false}
              data-active={active ? 'true' : undefined}
              style={{
                padding: '6px 12px',
                borderRadius: 0,
                fontSize: 13,
                textDecoration: 'none',
                border: '1px solid',
                background: active ? 'var(--bg-elev)' : 'transparent',
                borderColor: active ? 'var(--border-strong)' : 'var(--border)',
                color: active ? 'var(--fg)' : 'var(--fg-muted)',
              }}
            >
              {v.label}
            </Link>
          );
        })}
      </nav>

      {view === 'grouped' ? (
        <GroupedView clusters={clusters} />
      ) : (
        <Plane tilt="flat">
          {listing.submissions.length === 0 ? (
            <p
              style={{
                margin: 0,
                padding: '40px 24px',
                textAlign: 'center',
                color: 'var(--fg-muted)',
                fontSize: 14,
              }}
            >
              No submissions in this bucket.
            </p>
          ) : (
            <table
              data-testid="parser-submissions-table"
              style={{ width: '100%', borderCollapse: 'collapse', fontSize: 13 }}
            >
              <thead>
                <tr style={{ background: 'var(--bg-elev)' }}>
                  <Th>Shape hash</Th>
                  <Th>Shell tag</Th>
                  <Th align="right">Installs</Th>
                  <Th align="right">Occurrences</Th>
                  <Th>Last seen</Th>
                  <Th>Status</Th>
                </tr>
              </thead>
              <tbody>
                {listing.submissions.map((row) => (
                  <SubmissionRow key={row.id} row={row} />
                ))}
              </tbody>
            </table>
          )}
        </Plane>
      )}

      <nav
        aria-label="Parser-submissions pagination"
        style={{
          display: 'flex',
          justifyContent: 'flex-end',
          alignItems: 'center',
          gap: 12,
        }}
      >
        {olderHref ? (
          <Link href={olderHref} className="hp-btn hp-btn--ghost">
            Load more →
          </Link>
        ) : (
          <span
            className="hp-btn hp-btn--ghost"
            aria-disabled="true"
            style={{ opacity: 0.4, pointerEvents: 'none' }}
          >
            Load more →
          </span>
        )}
      </nav>
    </div>
  );
}

/**
 * Grouped (clustered) triage view: rows sharing a `coarse_shape` collapse
 * into one `<details>` block so a moderator can act on a whole family of
 * near-duplicate shapes at once. Native `<details>`/`<summary>` gives
 * expand/collapse with no client-side JS — this page stays a server
 * component. Clustering is computed over only the rows already fetched
 * for the current page/status (see `cluster.ts` for the growth-path note).
 */
function GroupedView({ clusters }: { clusters: Cluster[] }) {
  if (clusters.length === 0) {
    return (
      <Plane tilt="flat">
        <p
          style={{
            margin: 0,
            padding: '40px 24px',
            textAlign: 'center',
            color: 'var(--fg-muted)',
            fontSize: 14,
          }}
        >
          No submissions in this bucket.
        </p>
      </Plane>
    );
  }

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 12 }}>
      {clusters.map((cluster) => (
        <ClusterDetails key={cluster.coarseShape} cluster={cluster} />
      ))}
    </div>
  );
}

function ClusterDetails({ cluster }: { cluster: Cluster }) {
  const summaryLabel =
    cluster.representative.raw_example_preview ?? cluster.coarseShape;

  return (
    <details className="ss-card" style={{ padding: 0, overflow: 'hidden' }}>
      <summary
        style={{
          cursor: 'pointer',
          padding: '14px 18px',
          display: 'flex',
          justifyContent: 'space-between',
          alignItems: 'center',
          gap: 12,
        }}
      >
        <span
          style={{
            fontFamily:
              'var(--font-mono, ui-monospace, SFMono-Regular, monospace)',
            fontSize: 13,
            color: 'var(--fg)',
            overflow: 'hidden',
            textOverflow: 'ellipsis',
            whiteSpace: 'nowrap',
          }}
        >
          {summaryLabel}
        </span>
        <span
          style={{
            display: 'flex',
            gap: 10,
            fontSize: 12,
            color: 'var(--fg-muted)',
            flexShrink: 0,
          }}
        >
          <span>
            {cluster.members.length}{' '}
            {cluster.members.length === 1 ? 'shape' : 'shapes'}
          </span>
          <span>{cluster.totalOccurrences} occurrences</span>
        </span>
      </summary>
      <table style={{ width: '100%', borderCollapse: 'collapse', fontSize: 13 }}>
        <thead>
          <tr style={{ background: 'var(--bg-elev)' }}>
            <Th>Shape hash</Th>
            <Th>Shell tag</Th>
            <Th align="right">Installs</Th>
            <Th align="right">Occurrences</Th>
            <Th>Last seen</Th>
            <Th>Status</Th>
          </tr>
        </thead>
        <tbody>
          {cluster.members.map((row) => (
            <SubmissionRow key={row.id} row={row} />
          ))}
        </tbody>
      </table>
    </details>
  );
}

function SubmissionRow({ row }: { row: AdminParserSubmissionSummary }) {
  return (
    <tr style={{ borderTop: '1px solid var(--border)' }}>
      <Td>
        <Link
          href={`/admin/parser-submissions/${row.id}` as Route}
          prefetch={false}
          style={{
            fontFamily: 'var(--font-mono, ui-monospace, SFMono-Regular, monospace)',
            color: 'var(--fg)',
            textDecoration: 'none',
          }}
        >
          {row.shape_hash.length > 16
            ? `${row.shape_hash.slice(0, 16)}…`
            : row.shape_hash}
        </Link>
      </Td>
      <Td>
        <span style={{ color: 'var(--fg-muted)' }}>
          {row.shell_tag ?? '—'}
        </span>
      </Td>
      <Td align="right">{row.submitter_count}</Td>
      <Td align="right">{row.total_occurrence_count}</Td>
      <Td>
        <span
          style={{ color: 'var(--fg-muted)' }}
          title={row.last_submitted_at}
        >
          {relTime(row.last_submitted_at)}
        </span>
      </Td>
      <Td>
        <StatusPill status={row.status} />
      </Td>
    </tr>
  );
}

function StatusPill({ status }: { status: string }) {
  const map: Record<string, { fg: string; bg: string; label: string }> = {
    pending: { fg: 'var(--fg)', bg: 'var(--bg-elev)', label: 'Pending' },
    drafting: { fg: 'var(--fg)', bg: 'var(--bg-elev)', label: 'Drafting' },
    rule_written: {
      fg: 'var(--fg)',
      bg: 'var(--bg-elev)',
      label: 'Rule written',
    },
    dismissed: {
      fg: 'var(--fg-muted)',
      bg: 'transparent',
      label: 'Dismissed',
    },
  };
  const m = map[status] ?? {
    fg: 'var(--fg-muted)',
    bg: 'transparent',
    label: status,
  };
  return (
    <span
      style={{
        display: 'inline-block',
        padding: '2px 10px',
        borderRadius: 0,
        border: '1px solid var(--border)',
        background: m.bg,
        color: m.fg,
        fontSize: 12,
      }}
    >
      {m.label}
    </span>
  );
}

function Th({
  children,
  align,
}: {
  children: React.ReactNode;
  align?: 'left' | 'right';
}) {
  return (
    <th
      style={{
        textAlign: align ?? 'left',
        padding: '10px 14px',
        fontWeight: 600,
        color: 'var(--fg-muted)',
        fontSize: 11,
        letterSpacing: '0.06em',
        textTransform: 'uppercase',
      }}
    >
      {children}
    </th>
  );
}

function Td({
  children,
  align,
}: {
  children: React.ReactNode;
  align?: 'left' | 'right';
}) {
  return (
    <td
      style={{
        padding: '10px 14px',
        textAlign: align ?? 'left',
        verticalAlign: 'middle',
      }}
    >
      {children}
    </td>
  );
}
