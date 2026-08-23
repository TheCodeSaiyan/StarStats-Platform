/**
 * Admin · Audit log viewer.
 *
 * Replaces the Slice 5 placeholder. Lists `audit_log` rows from
 * GET /v1/admin/audit, with URL-driven filters for actor handle,
 * action name, and date range. Pagination is offset-based — the
 * server returns has_more so we know whether to render the "older"
 * link.
 *
 * Auth: the parent layout enforces moderator/admin role; this page
 * still calls getSession() for type narrowing + defensive redirect.
 */

import Link from 'next/link';
import type { Route } from 'next';
import { redirect } from 'next/navigation';
import {
  ApiCallError,
  getAdminAuditLog,
  type AuditEntryDto,
  type AuditListResponse,
} from '@/lib/api';
import { getSession } from '@/lib/session';
import { AdminPageHeader } from '../_components/AdminPageHeader';

interface SearchParams {
  actor?: string;
  action?: string;
  since?: string;
  until?: string;
  offset?: string;
}

const PAGE_SIZE = 50;

/**
 * `<input type="datetime-local">` submits values as `YYYY-MM-DDTHH:MM`
 * (or `YYYY-MM-DDTHH:MM:SS`) with NO timezone — the spec deliberately
 * treats it as wall-clock time. The Rust API deserializes the field
 * as `chrono::DateTime<Utc>` which demands RFC 3339, so we have to
 * append seconds (if missing) and a `Z` suffix before sending.
 *
 * The user's intent ("12:00 today") is interpreted as UTC by this
 * shim. A future iteration could read the browser's TZ offset and
 * convert properly, but UTC keeps the admin tool predictable across
 * sessions and is what an audit timeline wants anyway.
 *
 * Returns `null` when the input is unparseable so the API call can
 * skip the filter rather than 400.
 */
function normalizeDateTimeLocal(raw: string): string | null {
  if (!raw) return null;
  if (/Z$|[+-]\d{2}:\d{2}$/.test(raw)) return raw;
  if (/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}$/.test(raw)) return `${raw}Z`;
  if (/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}$/.test(raw)) return `${raw}:00Z`;
  return null;
}

export default async function AdminAuditPage(props: {
  searchParams: Promise<SearchParams>;
}) {
  const session = await getSession();
  if (!session) redirect('/auth/login?next=/admin/audit');

  const params = await props.searchParams;
  const actor = params.actor?.trim() ?? '';
  const action = params.action?.trim() ?? '';
  const since = params.since?.trim() ?? '';
  const until = params.until?.trim() ?? '';
  const offset = parseOffset(params.offset);

  const sinceIso = normalizeDateTimeLocal(since);
  const untilIso = normalizeDateTimeLocal(until);
  // If the user typed something that isn't a valid datetime (e.g.
  // pasted gibberish into the URL), drop the filter rather than 400.
  // We surface the dropped filter below the bar so they know.
  const sinceDropped = since !== '' && sinceIso === null;
  const untilDropped = until !== '' && untilIso === null;

  let result: AuditListResponse;
  let badRequestError: string | null = null;
  try {
    result = await getAdminAuditLog(session.token, {
      actor: actor || undefined,
      action: action || undefined,
      since: sinceIso ?? undefined,
      until: untilIso ?? undefined,
      limit: PAGE_SIZE,
      offset,
    });
  } catch (e) {
    if (e instanceof ApiCallError && e.status === 401) {
      redirect('/auth/login?next=/admin/audit');
    }
    if (e instanceof ApiCallError && e.status === 403) {
      redirect('/me');
    }
    // 400 means the API rejected one of our params. Show inline so
    // the page still renders (the filter bar stays usable).
    if (e instanceof ApiCallError && e.status === 400) {
      badRequestError = e.message;
      result = { entries: [], has_more: false };
    } else {
      throw e;
    }
  }

  const hasNewer = offset > 0;
  const newerOffset = Math.max(0, offset - PAGE_SIZE);
  const olderOffset = offset + PAGE_SIZE;
  const buildHref = (newOffset: number): Route => {
    const qs = new URLSearchParams();
    if (actor) qs.set('actor', actor);
    if (action) qs.set('action', action);
    if (since) qs.set('since', since);
    if (until) qs.set('until', until);
    if (newOffset > 0) qs.set('offset', String(newOffset));
    const suffix = qs.toString();
    return (suffix ? `/admin/audit?${suffix}` : '/admin/audit') as Route;
  };

  return (
    <div
      style={{ display: 'flex', flexDirection: 'column', gap: 20 }}
    >

      <AdminPageHeader
        eyebrow="Admin · audit log"
        title="Audit trail"
        lede={
          <>
            Every state-changing API call writes one hash-chained row. Filter
            by actor handle, action name, or timestamp range. The hash chain
            is verified on every insert at the database level.
          </>
        }
      />

      <FilterBar
        actor={actor}
        action={action}
        since={since}
        until={until}
      />

      {(badRequestError || sinceDropped || untilDropped) && (
        <div
          className="ss-badge"
          style={{
            alignSelf: 'flex-start',
            borderColor: 'var(--danger)',
            color: 'var(--danger)',
            fontSize: 12,
            lineHeight: 1.4,
            padding: '6px 10px',
          }}
        >
          {badRequestError
            ? `API rejected the request: ${badRequestError}`
            : `Ignored unparseable ${
                sinceDropped && untilDropped
                  ? 'since/until'
                  : sinceDropped
                    ? 'since'
                    : 'until'
              } filter — expected YYYY-MM-DDTHH:MM`}
        </div>
      )}

      <section className="ss-card" style={{ padding: 0, overflow: 'hidden' }}>
        {result.entries.length === 0 ? (
          <p
            style={{
              margin: 0,
              padding: '40px 24px',
              textAlign: 'center',
              color: 'var(--fg-muted)',
              fontSize: 14,
            }}
          >
            No audit entries match these filters.
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
                <Th width="80px">Seq</Th>
                <Th width="180px">When</Th>
                <Th width="180px">Actor</Th>
                <Th width="220px">Action</Th>
                <Th>Payload</Th>
              </tr>
            </thead>
            <tbody>
              {result.entries.map((entry) => (
                <AuditRow key={entry.seq} entry={entry} />
              ))}
            </tbody>
          </table>
        )}
      </section>

      <nav
        aria-label="Audit pagination"
        style={{
          display: 'flex',
          justifyContent: 'space-between',
          alignItems: 'center',
          gap: 12,
          flexWrap: 'wrap',
        }}
      >
        <span style={{ color: 'var(--fg-muted)', fontSize: 13 }}>
          {result.entries.length === 0
            ? 'Nothing on this page'
            : `Showing seqs ${result.entries[result.entries.length - 1]!.seq}` +
              ` – ${result.entries[0]!.seq}`}
        </span>
        <div style={{ display: 'flex', gap: 8 }}>
          {hasNewer ? (
            <Link href={buildHref(newerOffset)} className="ss-btn ss-btn--ghost">
              ← Newer
            </Link>
          ) : (
            <span
              className="ss-btn ss-btn--ghost"
              aria-disabled="true"
              style={{ opacity: 0.4, pointerEvents: 'none' }}
            >
              ← Newer
            </span>
          )}
          {result.has_more ? (
            <Link href={buildHref(olderOffset)} className="ss-btn ss-btn--ghost">
              Older →
            </Link>
          ) : (
            <span
              className="ss-btn ss-btn--ghost"
              aria-disabled="true"
              style={{ opacity: 0.4, pointerEvents: 'none' }}
            >
              Older →
            </span>
          )}
        </div>
      </nav>
    </div>
  );
}

function parseOffset(raw: string | undefined): number {
  if (!raw) return 0;
  const n = Number.parseInt(raw, 10);
  if (!Number.isFinite(n) || n < 0) return 0;
  return n;
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

function AuditRow({ entry }: { entry: AuditEntryDto }) {
  const prettyPayload = JSON.stringify(entry.payload, null, 2);
  const when = new Date(entry.occurred_at)
    .toISOString()
    .replace('T', ' ')
    .slice(0, 19);
  return (
    <tr style={{ borderBottom: '1px solid var(--border)' }}>
      <Td>
        <span className="mono" style={{ color: 'var(--fg-dim)' }}>
          #{entry.seq}
        </span>
      </Td>
      <Td>
        <span className="mono" style={{ fontSize: 12 }}>
          {when}
        </span>
      </Td>
      <Td>
        {entry.actor_handle ? (
          <Link
            href={(`/u/${encodeURIComponent(entry.actor_handle)}`) as Route}
            className="mono"
            style={{ color: 'var(--accent)' }}
          >
            {entry.actor_handle}
          </Link>
        ) : (
          <span style={{ color: 'var(--fg-dim)' }}>system</span>
        )}
      </Td>
      <Td>
        <span
          className="mono"
          style={{
            fontSize: 12,
            color: actionColor(entry.action),
            wordBreak: 'break-all',
          }}
        >
          {entry.action}
        </span>
      </Td>
      <Td>
        <pre
          className="mono"
          style={{
            margin: 0,
            fontSize: 11,
            color: 'var(--fg-muted)',
            background: 'transparent',
            whiteSpace: 'pre-wrap',
            wordBreak: 'break-word',
            maxHeight: 200,
            overflow: 'auto',
          }}
        >
          {prettyPayload}
        </pre>
      </Td>
    </tr>
  );
}

function Td({ children }: { children: React.ReactNode }) {
  return (
    <td style={{ padding: '10px 14px', verticalAlign: 'top' }}>{children}</td>
  );
}

/** Colour-code action namespaces so the eye can pick the danger ones
 *  (revoke, delete) out of a wall of greens. */
function actionColor(action: string): string {
  if (
    action.includes('revoke') ||
    action.includes('delete') ||
    action.includes('rejected') ||
    action.includes('forbidden')
  ) {
    return 'var(--danger)';
  }
  if (
    action.includes('grant') ||
    action.includes('accepted') ||
    action.includes('verified')
  ) {
    return 'var(--accent)';
  }
  return 'var(--fg-muted)';
}

function FilterBar({
  actor,
  action,
  since,
  until,
}: {
  actor: string;
  action: string;
  since: string;
  until: string;
}) {
  return (
    <form
      method="GET"
      action="/admin/audit"
      style={{
        display: 'grid',
        gridTemplateColumns: 'repeat(auto-fit, minmax(180px, 1fr))',
        gap: 8,
        alignItems: 'end',
      }}
    >
      <Field label="Actor handle" name="actor" defaultValue={actor} placeholder="e.g. alice" />
      <Field
        label="Action"
        name="action"
        defaultValue={action}
        placeholder="e.g. share.granted"
      />
      <Field
        label="Since"
        name="since"
        defaultValue={since}
        type="datetime-local"
      />
      <Field
        label="Until"
        name="until"
        defaultValue={until}
        type="datetime-local"
      />
      <div style={{ display: 'flex', gap: 8 }}>
        <button type="submit" className="ss-btn ss-btn--primary">
          Apply filters
        </button>
        <Link
          href={'/admin/audit' as Route}
          className="ss-btn ss-btn--ghost"
          style={{ textDecoration: 'none' }}
        >
          Reset
        </Link>
      </div>
    </form>
  );
}

function Field({
  label,
  name,
  defaultValue,
  placeholder,
  type = 'text',
}: {
  label: string;
  name: string;
  defaultValue: string;
  placeholder?: string;
  type?: 'text' | 'datetime-local';
}) {
  return (
    <label
      style={{
        display: 'flex',
        flexDirection: 'column',
        gap: 4,
        fontSize: 11,
        color: 'var(--fg-muted)',
        letterSpacing: '0.06em',
        textTransform: 'uppercase',
      }}
    >
      <span>{label}</span>
      <input
        type={type}
        name={name}
        defaultValue={defaultValue}
        placeholder={placeholder}
        spellCheck={false}
        autoComplete="off"
        className="mono"
        style={{
          padding: '8px 10px',
          background: 'var(--bg-elev)',
          border: '1px solid var(--border)',
          borderRadius: 0,
          color: 'var(--fg)',
          fontSize: 13,
          textTransform: 'none',
        }}
      />
    </label>
  );
}
