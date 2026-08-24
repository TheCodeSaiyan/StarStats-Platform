import { Plane } from 'holo';
/**
 * Admin · Sharing audit log.
 *
 * Thin wrapper around `/admin/audit`. Same data source and pagination
 * shape; the difference is the action filter is constrained to the
 * sharing-related set defined by the W3 audit:
 *
 *   - share.granted
 *   - share.revoked
 *   - share.scope_changed
 *   - share.viewed
 *   - share.reported
 *
 * Filtering strategy:
 *   - Actor handle: server-side via the `actor` query param, identical
 *     to /admin/audit.
 *   - Kind (action name): URL-driven dropdown. When set, passed to
 *     the API as `action`. When unset ("All sharing kinds"), we have
 *     to do client-side filtering: the backend doesn't support an
 *     IN-clause for action names, so the only correct way to combine
 *     N action filters in one query is to issue N requests in
 *     parallel and merge — see comment in the data-load block.
 *   - Date range: server-side via `since`/`until`, identical to
 *     /admin/audit.
 *
 * Pagination notes when "All sharing kinds" is selected:
 *   We pull `ALL_KIND_WINDOW` rows per action in parallel, merge,
 *   sort descending by seq, then slice the requested offset window.
 *   This means deep pagination is approximate — if one bucket
 *   overflows the per-bucket window we silently drop the tail. The
 *   page surfaces a hint explaining this. Single-kind filtering goes
 *   through the normal server pagination path and is exact.
 *
 * Date inputs use `datetime-local` which submits without a TZ; we
 * reuse the same normalisation trick as /admin/audit (treat as UTC,
 * append `Z`).
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
import { AdminPageHeader } from '../../_components/AdminPageHeader';

const SHARING_ACTIONS = [
  'share.granted',
  'share.revoked',
  'share.scope_changed',
  'share.viewed',
  'share.reported',
] as const;

type SharingAction = (typeof SHARING_ACTIONS)[number];

interface SearchParams {
  actor?: string;
  kind?: string;
  since?: string;
  until?: string;
  offset?: string;
}

const PAGE_SIZE = 50;
const ALL_KIND_WINDOW = 200;

/**
 * Coerce a `kind` query param into a known SharingAction, or return
 * null when the dropdown is on "All sharing kinds" (or someone pasted
 * a garbage value into the URL — we treat unknown == all).
 */
function parseKind(raw: string | undefined): SharingAction | null {
  if (!raw) return null;
  return (SHARING_ACTIONS as ReadonlyArray<string>).includes(raw)
    ? (raw as SharingAction)
    : null;
}

/**
 * Same datetime-local → RFC3339 shim as /admin/audit. Kept inline
 * because it's tiny and copy-paste makes the dependency obvious.
 */
function normalizeDateTimeLocal(raw: string): string | null {
  if (!raw) return null;
  if (/Z$|[+-]\d{2}:\d{2}$/.test(raw)) return raw;
  if (/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}$/.test(raw)) return `${raw}Z`;
  if (/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}$/.test(raw)) return `${raw}:00Z`;
  return null;
}

function parseOffset(raw: string | undefined): number {
  if (!raw) return 0;
  const n = Number.parseInt(raw, 10);
  if (!Number.isFinite(n) || n < 0) return 0;
  return n;
}

export default async function AdminSharingAuditPage(props: {
  searchParams: Promise<SearchParams>;
}) {
  const session = await getSession();
  if (!session) redirect('/auth/login?next=/admin/sharing/audit');

  const params = await props.searchParams;
  const actor = params.actor?.trim() ?? '';
  const kindRaw = params.kind?.trim() ?? '';
  const kind = parseKind(kindRaw);
  const since = params.since?.trim() ?? '';
  const until = params.until?.trim() ?? '';
  const offset = parseOffset(params.offset);

  const sinceIso = normalizeDateTimeLocal(since);
  const untilIso = normalizeDateTimeLocal(until);
  const sinceDropped = since !== '' && sinceIso === null;
  const untilDropped = until !== '' && untilIso === null;

  let result: AuditListResponse;
  let approxPagination = false;
  let badRequestError: string | null = null;

  try {
    if (kind) {
      // Single-kind path: standard server-paginated fetch.
      result = await getAdminAuditLog(session.token, {
        actor: actor || undefined,
        action: kind,
        since: sinceIso ?? undefined,
        until: untilIso ?? undefined,
        limit: PAGE_SIZE,
        offset,
      });
    } else {
      // All-sharing-kinds path: fan out one request per action,
      // merge client-side, then slice the offset window.
      //
      // We pull `ALL_KIND_WINDOW` rows per bucket. That's an upper
      // bound on how many merged rows we can serve; deep pagination
      // beyond ~ALL_KIND_WINDOW merged rows would require upgrading
      // the backend to accept multiple action filters in one query.
      const fetched = await Promise.all(
        SHARING_ACTIONS.map((action) =>
          getAdminAuditLog(session.token, {
            actor: actor || undefined,
            action,
            since: sinceIso ?? undefined,
            until: untilIso ?? undefined,
            limit: ALL_KIND_WINDOW,
            // Always start at 0 per bucket: we'll do the offset slice
            // after merging.
            offset: 0,
          }).then((r) => r.entries),
        ),
      );
      const merged = mergeAndSort(fetched);
      const totalMerged = merged.length;
      const sliceEnd = offset + PAGE_SIZE;
      result = {
        entries: merged.slice(offset, sliceEnd),
        has_more: sliceEnd < totalMerged,
      };
      approxPagination = true;
    }
  } catch (e) {
    if (e instanceof ApiCallError && e.status === 401) {
      redirect('/auth/login?next=/admin/sharing/audit');
    }
    if (e instanceof ApiCallError && e.status === 403) {
      redirect('/me');
    }
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
    if (kind) qs.set('kind', kind);
    if (since) qs.set('since', since);
    if (until) qs.set('until', until);
    if (newOffset > 0) qs.set('offset', String(newOffset));
    const suffix = qs.toString();
    return (suffix
      ? `/admin/sharing/audit?${suffix}`
      : '/admin/sharing/audit') as Route;
  };

  return (
    <div
      style={{ display: 'flex', flexDirection: 'column', gap: 20 }}
    >

      <AdminPageHeader
        eyebrow="Admin · sharing · audit"
        title="Sharing audit trail"
        lede={
          <>
            The audit log narrowed to the five sharing-related actions.
            Same hash-chained source as{' '}
            <Link
            href={'/admin/audit' as Route}
            style={{ color: 'var(--accent)' }}
            >
            /admin/audit
            </Link>
            ; this view just constrains the action filter.
          </>
        }
      />

      <nav
        aria-label="Sharing sub-views"
        style={{
          display: 'flex',
          gap: 8,
          flexWrap: 'wrap',
        }}
      >
        <Link
          href={'/admin/sharing' as Route}
          className="hp-btn hp-btn--ghost"
          style={{ textDecoration: 'none' }}
        >
          ← Overview
        </Link>
      </nav>

      <FilterBar
        actor={actor}
        kind={kindRaw}
        since={since}
        until={until}
      />

      {(badRequestError || sinceDropped || untilDropped) && (
        <div
          className="hp-chip"
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

      {approxPagination && (
        <div
          className="hp-chip"
          style={{
            alignSelf: 'flex-start',
            fontSize: 11,
            color: 'var(--fg-muted)',
            borderColor: 'var(--border)',
            padding: '6px 10px',
            lineHeight: 1.4,
          }}
        >
          Showing merged rows across all five sharing actions. Deep
          pagination is approximate — pick a single kind for exact
          server-paginated results.
        </div>
      )}

      <Plane tilt="flat">
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
            Scope is clear — no sharing audit entries match these
            filters.
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
                <Th width="180px">Kind</Th>
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
      </Plane>

      <nav
        aria-label="Sharing-audit pagination"
        style={{
          display: 'flex',
          justifyContent: 'space-between',
          alignItems: 'center',
          gap: 12,
          flexWrap: 'wrap',
        }}
      >
        <span className="hp-fine">
          {result.entries.length === 0
            ? 'Nothing on this page'
            : `Showing seqs ${
                result.entries[result.entries.length - 1]!.seq
              } – ${result.entries[0]!.seq}`}
        </span>
        <div style={{ display: 'flex', gap: 8 }}>
          {hasNewer ? (
            <Link
              href={buildHref(newerOffset)}
              className="hp-btn hp-btn--ghost"
            >
              ← Newer
            </Link>
          ) : (
            <span
              className="hp-btn hp-btn--ghost"
              aria-disabled="true"
              style={{ opacity: 0.4, pointerEvents: 'none' }}
            >
              ← Newer
            </span>
          )}
          {result.has_more ? (
            <Link
              href={buildHref(olderOffset)}
              className="hp-btn hp-btn--ghost"
            >
              Older →
            </Link>
          ) : (
            <span
              className="hp-btn hp-btn--ghost"
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

/**
 * Merge N arrays of entries already sorted descending by seq into
 * one stable descending list. We use a simple flat-then-sort because
 * total size is bounded (5 buckets × ALL_KIND_WINDOW = 1000 rows
 * max). A k-way merge would be marginally faster but adds code at
 * no perceptible gain.
 *
 * Built immutably: `.flat()` returns a new array, `.slice()` clones
 * before sorting so the bucket arrays aren't mutated.
 */
function mergeAndSort(
  buckets: ReadonlyArray<ReadonlyArray<AuditEntryDto>>,
): ReadonlyArray<AuditEntryDto> {
  return buckets
    .flat()
    .slice()
    .sort((a, b) => b.seq - a.seq);
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
    <td style={{ padding: '10px 14px', verticalAlign: 'top' }}>
      {children}
    </td>
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
            href={
              (`/u/${encodeURIComponent(entry.actor_handle)}`) as Route
            }
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

/** Colour-code revoke/report as danger, grant/view as accent. */
function actionColor(action: string): string {
  if (action === 'share.revoked' || action === 'share.reported') {
    return 'var(--danger)';
  }
  if (action === 'share.granted' || action === 'share.viewed') {
    return 'var(--accent)';
  }
  return 'var(--fg-muted)';
}

function FilterBar({
  actor,
  kind,
  since,
  until,
}: {
  actor: string;
  kind: string;
  since: string;
  until: string;
}) {
  return (
    <form
      method="GET"
      action="/admin/sharing/audit"
      style={{
        display: 'grid',
        gridTemplateColumns: 'repeat(auto-fit, minmax(180px, 1fr))',
        gap: 8,
        alignItems: 'end',
      }}
    >
      <Field
        label="Actor handle"
        name="actor"
        defaultValue={actor}
        placeholder="e.g. alice"
      />
      <KindSelect kind={kind} />
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
        <button type="submit" className="hp-btn hp-btn--primary">
          Apply filters
        </button>
        <Link
          href={'/admin/sharing/audit' as Route}
          className="hp-btn hp-btn--ghost"
          style={{ textDecoration: 'none' }}
        >
          Reset
        </Link>
      </div>
    </form>
  );
}

function KindSelect({ kind }: { kind: string }) {
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
      <span>Kind</span>
      <select
        name="kind"
        defaultValue={kind}
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
      >
        <option value="">All sharing kinds</option>
        {SHARING_ACTIONS.map((a) => (
          <option key={a} value={a}>
            {a}
          </option>
        ))}
      </select>
    </label>
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

// Force dynamic: audit data is non-cacheable.
export const dynamic = 'force-dynamic';
