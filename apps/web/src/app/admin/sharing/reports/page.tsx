/**
 * Admin · Sharing · Reports queue (audit v2 §05 "Reports queue").
 *
 * Moderator landing page for share-report triage. Defaults to the
 * unresolved `open` bucket; a tab strip flips between
 * `open | dismissed | share_revoked | user_suspended | all`. Each
 * row carries three resolution buttons (Dismiss / Revoke / Suspend);
 * each posts to `resolveShareReportAction` with a different hidden
 * `outcome` value.
 *
 * Auth: parent `/admin/layout.tsx` gates the subtree on
 * moderator/admin role. We still call `getSession()` here for type
 * narrowing + a defensive redirect.
 */

import Link from 'next/link';
import type { Route } from 'next';
import { redirect } from 'next/navigation';
import {
  ApiCallError,
  getAdminSharingReports,
  type ShareReportListResponse,
  type ShareReportRowDto,
} from '@/lib/api';
import { getSession } from '@/lib/session';
import { ConfirmSubmitButton } from '@/components/forms/ConfirmSubmitButton';
import { resolveShareReportAction } from './actions';

type StatusFilter =
  | 'open'
  | 'dismissed'
  | 'share_revoked'
  | 'user_suspended'
  | 'all';

const STATUS_FILTERS: ReadonlyArray<{ id: StatusFilter; label: string }> = [
  { id: 'open', label: 'Open' },
  { id: 'dismissed', label: 'Dismissed' },
  { id: 'share_revoked', label: 'Share revoked' },
  { id: 'user_suspended', label: 'User suspended' },
  { id: 'all', label: 'All' },
];

function parseStatus(s: string | undefined): StatusFilter {
  switch (s) {
    case 'dismissed':
    case 'share_revoked':
    case 'user_suspended':
    case 'all':
      return s;
    default:
      return 'open';
  }
}

function formatReason(r: string): string {
  switch (r) {
    case 'abuse':
      return 'Abuse';
    case 'spam':
      return 'Spam';
    case 'data_misuse':
      return 'Data misuse';
    case 'other':
      return 'Other';
    default:
      return r;
  }
}

function formatStatus(s: string): string {
  switch (s) {
    case 'open':
      return 'Open';
    case 'dismissed':
      return 'Dismissed';
    case 'share_revoked':
      return 'Share revoked';
    case 'user_suspended':
      return 'User suspended';
    default:
      return s;
  }
}

export default async function AdminSharingReportsPage({
  searchParams,
}: {
  searchParams: Promise<{ status?: string }>;
}) {
  const session = await getSession();
  if (!session) redirect('/auth/login?next=/admin/sharing/reports');

  const params = await searchParams;
  const status = parseStatus(params.status);

  let data: ShareReportListResponse;
  try {
    data = await getAdminSharingReports(session.token, {
      status,
      limit: 100,
    });
  } catch (e) {
    if (e instanceof ApiCallError && e.status === 401) {
      redirect('/auth/login?next=/admin/sharing/reports');
    }
    if (e instanceof ApiCallError && e.status === 403) {
      redirect('/me');
    }
    throw e;
  }

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 16 }}>
      <header style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
        <h1 style={{ margin: 0 }}>Share reports</h1>
        <p
          style={{
            margin: 0,
            color: 'var(--fg-muted)',
            fontSize: 14,
          }}
        >
          Moderator triage queue for share-misuse complaints. Default
          filter shows unresolved reports.
        </p>
      </header>

      <nav
        aria-label="Sharing sub-sections"
        style={{ display: 'flex', gap: 12, fontSize: 13 }}
      >
        <Link
          href={'/admin/sharing' as Route}
          prefetch={false}
          style={{ color: 'var(--fg-muted)' }}
        >
          Overview
        </Link>
        <Link
          href={'/admin/sharing/audit' as Route}
          prefetch={false}
          style={{ color: 'var(--fg-muted)' }}
        >
          Audit log
        </Link>
        <span style={{ color: 'var(--fg)' }} aria-current="page">
          Reports
        </span>
      </nav>

      <nav
        aria-label="Status filter"
        style={{ display: 'flex', flexWrap: 'wrap', gap: 6 }}
      >
        {STATUS_FILTERS.map((f) => {
          const active = f.id === status;
          const href =
            f.id === 'open'
              ? ('/admin/sharing/reports' as Route)
              : (`/admin/sharing/reports?status=${f.id}` as Route);
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
                borderColor: active
                  ? 'var(--border-strong)'
                  : 'var(--border)',
                background: active ? 'var(--bg-elev)' : 'transparent',
                color: active ? 'var(--fg)' : 'var(--fg-muted)',
              }}
            >
              {f.label}
            </Link>
          );
        })}
      </nav>

      {data.items.length === 0 ? (
        <p
          style={{
            margin: 0,
            padding: '16px 0',
            color: 'var(--fg-muted)',
          }}
        >
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
          {data.items.map((row) => (
            <ReportRow key={row.id} row={row} />
          ))}
        </ul>
      )}
    </div>
  );
}

function ReportRow({ row }: { row: ShareReportRowDto }) {
  const isOpen = row.status === 'open';
  return (
    <li
      className="ss-card"
      style={{
        padding: 14,
        display: 'flex',
        flexDirection: 'column',
        gap: 10,
      }}
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
          <strong style={{ fontSize: 14 }}>
            {row.owner_handle} → {row.recipient_handle}
          </strong>
          <span style={{ fontSize: 12, color: 'var(--fg-muted)' }}>
            Reported by {row.reporter_handle} · {formatReason(row.reason)}
          </span>
        </div>
        <span
          className="ss-badge"
          data-status={row.status}
          style={{ fontSize: 11 }}
        >
          {formatStatus(row.status)}
        </span>
      </header>

      {row.details && (
        <p
          style={{
            margin: 0,
            padding: 10,
            background: 'var(--bg-sunken)',
            borderRadius: 0,
            fontSize: 13,
            whiteSpace: 'pre-wrap',
          }}
        >
          {row.details}
        </p>
      )}

      <footer
        style={{
          display: 'flex',
          flexDirection: 'column',
          gap: 8,
          fontSize: 12,
          color: 'var(--fg-muted)',
        }}
      >
        <span>
          Filed {new Date(row.created_at).toLocaleString()}
          {row.resolved_at && (
            <>
              {' · resolved '}
              {new Date(row.resolved_at).toLocaleString()}
              {row.resolved_by && <> by {row.resolved_by}</>}
            </>
          )}
        </span>
        {row.resolution_note && (
          <span style={{ fontStyle: 'italic' }}>
            Moderator note: {row.resolution_note}
          </span>
        )}
      </footer>

      {isOpen && (
        <form
          action={resolveShareReportAction}
          style={{
            display: 'flex',
            flexDirection: 'column',
            gap: 8,
            borderTop: '1px solid var(--border)',
            paddingTop: 10,
          }}
        >
          <input type="hidden" name="id" value={row.id} />
          <label
            style={{
              display: 'flex',
              flexDirection: 'column',
              gap: 4,
              fontSize: 12,
            }}
          >
            <span style={{ color: 'var(--fg-muted)' }}>
              Optional moderator note (≤ 500 chars)
            </span>
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
            <ConfirmSubmitButton
              name="outcome"
              value="dismissed"
              className="ss-btn"
              pendingLabel="Working…"
            >
              Dismiss
            </ConfirmSubmitButton>
            <ConfirmSubmitButton
              name="outcome"
              value="share_revoked"
              className="ss-btn"
              confirm="Revoke this share? The recipient will lose access immediately."
              pendingLabel="Working…"
            >
              Revoke share
            </ConfirmSubmitButton>
            <ConfirmSubmitButton
              name="outcome"
              value="user_suspended"
              className="ss-btn ss-btn--danger"
              confirm="Suspend this owner? Every capability is blocked - ingest, sharing, public profile and submissions - and all of their existing shares are revoked. Revoked shares are NOT restored if you reinstate them later."
              pendingLabel="Working…"
            >
              Suspend owner
            </ConfirmSubmitButton>
          </div>
        </form>
      )}
    </li>
  );
}
