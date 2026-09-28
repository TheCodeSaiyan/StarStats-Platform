/**
 * Admin · Chat · Reports queue.
 *
 * Chat is end-to-end encrypted, so StarStats never holds messages. A report
 * is the exception: the reporter chose which messages to reveal, and they
 * are shown here and nowhere else. Matrix has no message franking, so a
 * revealed message cannot prove who sent it; the page says so above every
 * report, and decisions should weigh it (corroboration, history, several
 * reporters).
 *
 * "Restrict from chat" adds a chat restriction to whatever the player
 * already has; "Suspend" blocks everything. Both take the player out of
 * every chat room at once.
 *
 * Auth: `/admin/layout.tsx` gates the subtree on moderator/admin.
 */

import Link from 'next/link';
import type { Route } from 'next';
import { redirect } from 'next/navigation';
import { ApiCallError, getAdminChatReports, type ChatReport } from '@/lib/api';
import { getSession } from '@/lib/session';
import { ConfirmSubmitButton } from '@/components/forms/ConfirmSubmitButton';
import { resolveChatReportAction } from './actions';

type StatusFilter = 'open' | 'dismissed' | 'chat_restricted' | 'user_suspended' | 'all';

const STATUS_FILTERS: ReadonlyArray<{ id: StatusFilter; label: string }> = [
  { id: 'open', label: 'Open' },
  { id: 'dismissed', label: 'Dismissed' },
  { id: 'chat_restricted', label: 'Restricted from chat' },
  { id: 'user_suspended', label: 'Suspended' },
  { id: 'all', label: 'All' },
];

const REASONS: Record<string, string> = {
  harassment: 'Harassment or abuse',
  spam: 'Spam',
  scam: 'Scam or phishing',
  illegal_content: 'Illegal content',
  other: 'Other',
};

function parseStatus(s: string | undefined): StatusFilter {
  return STATUS_FILTERS.some((f) => f.id === s) ? (s as StatusFilter) : 'open';
}

export default async function AdminChatReportsPage({
  searchParams,
}: {
  searchParams: Promise<{ status?: string }>;
}) {
  const session = await getSession();
  if (!session) redirect('/auth/login?next=/admin/chat/reports');
  const status = parseStatus((await searchParams).status);

  let reports: ChatReport[];
  try {
    reports = (await getAdminChatReports(session.token, status)).reports;
  } catch (e) {
    if (e instanceof ApiCallError && e.status === 401) {
      redirect('/auth/login?next=/admin/chat/reports');
    }
    if (e instanceof ApiCallError && e.status === 403) redirect('/me');
    throw e;
  }

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 16 }}>
      <header style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
        <h1 style={{ margin: 0 }}>Chat reports</h1>
        <p style={{ margin: 0, color: 'var(--fg-muted)', fontSize: 14 }}>
          Players reported in chat, with the messages the reporter chose to reveal. Chat is
          encrypted end to end, so these are the only messages StarStats ever sees.
        </p>
      </header>

      <nav aria-label="Status filter" style={{ display: 'flex', flexWrap: 'wrap', gap: 6 }}>
        {STATUS_FILTERS.map((f) => {
          const active = f.id === status;
          const href =
            f.id === 'open'
              ? ('/admin/chat/reports' as Route)
              : (`/admin/chat/reports?status=${f.id}` as Route);
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

function ReportRow({ report }: { report: ChatReport }) {
  return (
    <li
      className="ss-card"
      data-testid="chat-report"
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
          <strong style={{ fontSize: 14 }}>@{report.reported_handle}</strong>
          <span className="hp-kvlabel">
            Reported by {report.reporter_handle} · {REASONS[report.reason] ?? report.reason}
          </span>
        </div>
        <span className="hp-chip" data-status={report.status} style={{ fontSize: 11 }}>
          {STATUS_FILTERS.find((f) => f.id === report.status)?.label ?? report.status}
        </span>
      </header>

      <p style={{ margin: 0, fontSize: 12, color: 'var(--fg-muted)' }}>
        Revealed by the reporter. These cannot be proven to come from @{report.reported_handle}:
        weigh them with other evidence.
      </p>
      <ol
        style={{
          margin: 0,
          padding: 10,
          listStyle: 'none',
          background: 'var(--bg-sunken)',
          fontSize: 13,
          display: 'flex',
          flexDirection: 'column',
          gap: 4,
        }}
      >
        {report.messages.map((m) => (
          <li key={m.event_id}>
            <span style={{ color: 'var(--fg-muted)' }}>
              {new Date(m.sent_at).toLocaleString()}{' '}
            </span>
            <span style={{ whiteSpace: 'pre-wrap' }}>{m.text}</span>
          </li>
        ))}
      </ol>

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
          action={resolveChatReportAction}
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
              value="chat_restricted"
              className="hp-btn"
              confirm={`Restrict @${report.reported_handle} from chat? They are removed from every chat room at once; their other access is unchanged.`}
              pendingLabel="Working…"
            >
              Restrict from chat
            </ConfirmSubmitButton>
            <ConfirmSubmitButton
              name="outcome"
              value="user_suspended"
              className="hp-btn hp-btn--danger"
              confirm={`Suspend @${report.reported_handle}? Every capability is blocked, chat included, until a moderator lifts it.`}
              pendingLabel="Working…"
            >
              Suspend
            </ConfirmSubmitButton>
          </div>
        </form>
      ) : null}
    </li>
  );
}
