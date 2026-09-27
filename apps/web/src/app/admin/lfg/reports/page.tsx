/**
 * Admin · Looking for Group · Reports queue.
 *
 * Moderator triage for reported LFG posts. Defaults to `open`; a tab strip
 * flips between the resolutions. Each report shows the post as it was when
 * reported (the snapshot), because the post itself may have expired since.
 * Resolving as "remove post" takes it down; "suspend host" also restricts
 * the host's account, as the share-report queue does.
 *
 * Auth: `/admin/layout.tsx` gates the subtree on moderator/admin.
 */

import Link from 'next/link';
import type { Route } from 'next';
import { redirect } from 'next/navigation';
import { ApiCallError, getAdminLfgReports, type LfgReport } from '@/lib/api';
import { activityLabel } from '@/lib/lfg';
import { getSession } from '@/lib/session';
import { ConfirmSubmitButton } from '@/components/forms/ConfirmSubmitButton';
import { resolveLfgReportAction } from './actions';

type StatusFilter = 'open' | 'dismissed' | 'post_removed' | 'user_suspended' | 'all';

const STATUS_FILTERS: ReadonlyArray<{ id: StatusFilter; label: string }> = [
  { id: 'open', label: 'Open' },
  { id: 'dismissed', label: 'Dismissed' },
  { id: 'post_removed', label: 'Post removed' },
  { id: 'user_suspended', label: 'Host suspended' },
  { id: 'all', label: 'All' },
];

const REASONS: Record<string, string> = {
  abuse: 'Abusive or harassing',
  spam: 'Spam or advertising',
  illegal_content: 'Illegal content',
  other: 'Other',
};

function parseStatus(s: string | undefined): StatusFilter {
  return STATUS_FILTERS.some((f) => f.id === s) ? (s as StatusFilter) : 'open';
}

export default async function AdminLfgReportsPage({
  searchParams,
}: {
  searchParams: Promise<{ status?: string }>;
}) {
  const session = await getSession();
  if (!session) redirect('/auth/login?next=/admin/lfg/reports');
  const status = parseStatus((await searchParams).status);

  let reports: LfgReport[];
  try {
    reports = (await getAdminLfgReports(session.token, status)).reports;
  } catch (e) {
    if (e instanceof ApiCallError && e.status === 401) {
      redirect('/auth/login?next=/admin/lfg/reports');
    }
    if (e instanceof ApiCallError && e.status === 403) redirect('/me');
    throw e;
  }

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 16 }}>
      <header style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
        <h1 style={{ margin: 0 }}>LFG reports</h1>
        <p style={{ margin: 0, color: 'var(--fg-muted)', fontSize: 14 }}>
          Reported Looking for Group posts. Each shows the post as it was when reported.
        </p>
      </header>

      <nav aria-label="Status filter" style={{ display: 'flex', flexWrap: 'wrap', gap: 6 }}>
        {STATUS_FILTERS.map((f) => {
          const active = f.id === status;
          const href =
            f.id === 'open'
              ? ('/admin/lfg/reports' as Route)
              : (`/admin/lfg/reports?status=${f.id}` as Route);
          return (
            <Link
              key={f.id}
              href={href}
              prefetch={false}
              data-active={active ? 'true' : undefined}
              style={{
                padding: '6px 12px',
                borderRadius: 0,
                fontSize: 12,
                textDecoration: 'none',
                border: '1px solid',
                borderColor: active ? 'var(--border-strong)' : 'var(--border)',
                background: active ? 'var(--bg-elev)' : 'transparent',
                color: active ? 'var(--fg)' : 'var(--fg-muted)',
              }}
            >
              {f.label}
            </Link>
          );
        })}
      </nav>

      {reports.length === 0 ? (
        <p style={{ margin: 0, padding: '16px 0', color: 'var(--fg-muted)' }}>
          Nothing in this bucket.
        </p>
      ) : (
        <ul
          style={{
            listStyle: 'none',
            margin: 0,
            padding: 0,
            display: 'flex',
            flexDirection: 'column',
            gap: 12,
          }}
        >
          {reports.map((r) => (
            <ReportRow key={r.id} report={r} />
          ))}
        </ul>
      )}
    </div>
  );
}

function ReportRow({ report }: { report: LfgReport }) {
  const snap = (report.post_snapshot ?? {}) as {
    activity?: string;
    system?: string | null;
    location?: string | null;
    ship?: string | null;
    note?: string | null;
  };
  const summary = [
    snap.activity ? activityLabel(snap.activity) : null,
    snap.system,
    snap.location,
    snap.ship,
  ]
    .filter(Boolean)
    .join(' · ');
  return (
    <li
      className="ss-card"
      data-testid="lfg-report"
      style={{ padding: 14, display: 'flex', flexDirection: 'column', gap: 10 }}
    >
      <header
        style={{
          display: 'flex',
          justifyContent: 'space-between',
          alignItems: 'baseline',
          gap: 12,
          flexWrap: 'wrap',
        }}
      >
        <div style={{ display: 'flex', flexDirection: 'column', gap: 2 }}>
          <strong style={{ fontSize: 14 }}>Post by {report.host_handle}</strong>
          <span className="hp-kvlabel">
            Reported by {report.reporter_handle} · {REASONS[report.reason] ?? report.reason}
          </span>
        </div>
        <span className="hp-chip" data-status={report.status} style={{ fontSize: 11 }}>
          {STATUS_FILTERS.find((f) => f.id === report.status)?.label ?? report.status}
        </span>
      </header>

      <div
        style={{
          padding: 10,
          background: 'var(--bg-sunken)',
          fontSize: 13,
          display: 'flex',
          flexDirection: 'column',
          gap: 4,
        }}
      >
        <span>{summary || 'No details'}</span>
        {snap.note ? <span style={{ whiteSpace: 'pre-wrap' }}>“{snap.note}”</span> : null}
      </div>

      {report.details ? (
        <p style={{ margin: 0, fontSize: 13, whiteSpace: 'pre-wrap' }}>
          Reporter says: {report.details}
        </p>
      ) : null}

      <footer style={{ fontSize: 12, color: 'var(--fg-muted)' }}>
        Filed {new Date(report.created_at).toLocaleString()}
        {report.resolved_at ? (
          <>
            {' · resolved '}
            {new Date(report.resolved_at).toLocaleString()}
            {report.resolved_by ? <> by {report.resolved_by}</> : null}
          </>
        ) : null}
        {report.resolution_note ? (
          <span style={{ display: 'block', fontStyle: 'italic' }}>
            Moderator note: {report.resolution_note}
          </span>
        ) : null}
      </footer>

      {report.status === 'open' ? (
        <form
          action={resolveLfgReportAction}
          style={{
            display: 'flex',
            flexDirection: 'column',
            gap: 8,
            borderTop: '1px solid var(--border)',
            paddingTop: 10,
          }}
        >
          <input type="hidden" name="id" value={report.id} />
          <label style={{ display: 'flex', flexDirection: 'column', gap: 4, fontSize: 12 }}>
            <span style={{ color: 'var(--fg-muted)' }}>Optional moderator note (≤ 500 chars)</span>
            <textarea
              name="note"
              rows={2}
              maxLength={500}
              style={{
                resize: 'vertical',
                fontFamily: 'inherit',
                fontSize: 13,
                padding: 6,
                background: 'var(--bg-elev)',
                color: 'var(--fg)',
                border: '1px solid var(--border)',
                borderRadius: 0,
              }}
            />
          </label>
          <div style={{ display: 'flex', gap: 8, flexWrap: 'wrap' }}>
            <ConfirmSubmitButton name="outcome" value="dismissed" className="hp-btn" pendingLabel="Working…">
              Dismiss
            </ConfirmSubmitButton>
            <ConfirmSubmitButton
              name="outcome"
              value="post_removed"
              className="hp-btn"
              confirm="Take this post down? It leaves the board at once."
              pendingLabel="Working…"
            >
              Remove post
            </ConfirmSubmitButton>
            <ConfirmSubmitButton
              name="outcome"
              value="user_suspended"
              className="hp-btn hp-btn--danger"
              confirm="Remove the post and suspend the host? Every capability is blocked - ingest, sharing, public profile and submissions - until a moderator lifts it."
              pendingLabel="Working…"
            >
              Remove and suspend host
            </ConfirmSubmitButton>
          </div>
        </form>
      ) : null}
    </li>
  );
}
