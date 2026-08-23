/**
 * Per-entity event history page — all events ever tagged with a given
 * (kind, id) for one user, across every session.
 *
 * Server component — fetches `/v1/users/{handle}/entities/{kind}/{id}`
 * with the visitor's bearer token. Same `share_event_timeline` gate
 * as the entity index; 401/403/404 all collapse to a generic
 * "not available" render.
 *
 * Body layout:
 *   * Page header — entity kind pill + display name + stats trio.
 *   * Event list — chronologically ordered, folded via
 *     `foldAdjacentSameKey`. Section dividers drawn between events
 *     whose preceding session id (from `session_breakdown`) differs,
 *     so the reader can see which session each cluster belongs to.
 *   * "Load more" cursor pagination when `next_after` is present.
 */

import Link from 'next/link';
import type { Route } from 'next';
import {
  ApiCallError,
  getEntityHistory,
  type EntityHistoryResponse,
} from '@/lib/api';
import { logger } from '@/lib/logger';
import { getSession } from '@/lib/session';
import { InferredBadge } from '@/components/InferredBadge';
import {
  ENTITY_KINDS,
  foldAdjacentSameKey,
  labelForEntityKind,
  rowTitleForEnvelope,
  type TimelineRow,
} from '@/lib/timeline-metadata';

export const metadata = { title: "Entity" };

interface PageProps {
  params: Promise<{ handle: string; kind: string; id: string }>;
  searchParams: Promise<{ after?: string }>;
}

const KIND_SET: ReadonlySet<string> = new Set(ENTITY_KINDS);

/**
 * Discriminator used to distinguish the success and forbidden
 * responses from `loadHistory`. We can't reuse a `kind` field on the
 * error case because the success body itself has a `kind` field —
 * TypeScript's narrowing would collapse both branches.
 */
interface ForbiddenSentinel {
  forbidden: true;
}

async function loadHistory(
  bearer: string,
  handle: string,
  kind: string,
  id: string,
  after: string | undefined,
): Promise<EntityHistoryResponse | ForbiddenSentinel> {
  try {
    return await getEntityHistory(bearer, handle, kind, id, { after });
  } catch (e) {
    if (
      e instanceof ApiCallError &&
      (e.status === 401 || e.status === 403 || e.status === 404)
    ) {
      return { forbidden: true };
    }
    logger.error({ err: e }, 'entity history fetch failed');
    return { forbidden: true };
  }
}

export default async function EntityHistoryPage(props: PageProps) {
  const { handle, kind, id } = await props.params;
  const search = await props.searchParams;

  // Reject unknown kinds before the network round-trip. Same closed
  // vocabulary as the server's `validate_kind`. A typo'd link
  // collapses to the same "not available" state the 404 path uses.
  if (!KIND_SET.has(kind)) {
    return <EntityUnavailable handle={handle} />;
  }

  const session = await getSession();
  if (!session) {
    return <EntityUnavailable handle={handle} />;
  }

  const result = await loadHistory(
    session.token,
    handle,
    kind,
    id,
    search.after,
  );
  if ('forbidden' in result) {
    return <EntityUnavailable handle={handle} />;
  }

  const events = result.events;
  const totalEvents = events.length;
  // Session count comes from the breakdown the API returns.
  const sessionCount = result.session_breakdown.length;
  // Derive span from the first / last event timestamps. Falls back to
  // the empty string when no event carried a parseable timestamp;
  // the page just shows '—' in that case.
  const firstTs =
    events.length > 0 ? (events[0]!.event?.timestamp ?? '') : '';
  const lastTs =
    events.length > 0
      ? (events[events.length - 1]!.event?.timestamp ?? '')
      : '';

  const rows = foldAdjacentSameKey(events);

  return (
    <div
      className="ss-screen-enter"
      style={{ display: 'flex', flexDirection: 'column', gap: 20 }}
    >
      <header>
        <div className="ss-eyebrow" style={{ marginBottom: 8 }}>
          {labelForEntityKind(result.kind)}
        </div>
        <h1
          style={{
            margin: 0,
            fontSize: 28,
            fontWeight: 600,
            letterSpacing: '-0.02em',
          }}
        >
          {result.display_name}
        </h1>
        <div
          className="mono"
          style={{
            marginTop: 6,
            fontSize: 12,
            color: 'var(--fg-dim)',
            overflow: 'hidden',
            textOverflow: 'ellipsis',
            whiteSpace: 'nowrap',
          }}
        >
          {result.id}
        </div>
        <div
          data-testid="entity-stats"
          style={{
            marginTop: 14,
            display: 'flex',
            gap: 18,
            flexWrap: 'wrap',
            fontSize: 13,
            color: 'var(--fg-muted)',
          }}
        >
          <span>
            <span style={{ color: 'var(--fg)' }}>
              {totalEvents.toLocaleString()}
            </span>{' '}
            event{totalEvents === 1 ? '' : 's'}
            {result.next_after ? ' (more)' : ''}
          </span>
          <span>
            <span style={{ color: 'var(--fg)' }}>
              {sessionCount.toLocaleString()}
            </span>{' '}
            session{sessionCount === 1 ? '' : 's'}
          </span>
          <span
            className="mono"
            style={{ fontSize: 11 }}
            suppressHydrationWarning
          >
            {firstTs || '—'} → {lastTs || '—'}
          </span>
        </div>
        <div
          style={{
            marginTop: 14,
            display: 'flex',
            gap: 10,
            alignItems: 'center',
            flexWrap: 'wrap',
          }}
        >
          <Link
            href={
              (`/u/${encodeURIComponent(handle)}/entities`) as Route
            }
            className="ss-btn ss-btn--ghost"
            style={{ textDecoration: 'none' }}
          >
            ← Back to entities
          </Link>
        </div>
      </header>

      {events.length === 0 ? (
        <EmptyEntity />
      ) : (
        <>
          <SessionBreakdown
            handle={handle}
            buckets={result.session_breakdown}
          />
          <TimelineList rows={rows} />
        </>
      )}

      {result.next_after && (
        <LoadMoreNotice
          handle={handle}
          kind={result.kind}
          id={result.id}
          afterCursor={result.next_after}
        />
      )}
    </div>
  );
}

function EntityUnavailable({ handle }: { handle: string }) {
  return (
    <div
      className="ss-screen-enter"
      style={{ display: 'flex', flexDirection: 'column', gap: 20 }}
      data-testid="entity-history-forbidden"
    >
      <header>
        <div className="ss-eyebrow" style={{ marginBottom: 8 }}>
          Entity
        </div>
        <h1
          style={{
            margin: 0,
            fontSize: 28,
            fontWeight: 600,
            letterSpacing: '-0.02em',
          }}
        >
          Entity not available
        </h1>
        <p
          style={{
            margin: '6px 0 0',
            color: 'var(--fg-muted)',
            fontSize: 14,
          }}
        >
          This entity is either private or not shared with you.
        </p>
        <p style={{ marginTop: 14 }}>
          <Link
            href={(`/u/${encodeURIComponent(handle)}`) as Route}
            className="ss-btn ss-btn--ghost"
            style={{ textDecoration: 'none' }}
          >
            ← Back to profile
          </Link>
        </p>
      </header>
    </div>
  );
}

function EmptyEntity() {
  return (
    <section className="ss-card">
      <div style={{ padding: '24px', textAlign: 'center' }}>
        <p style={{ margin: 0, color: 'var(--fg-muted)', fontSize: 14 }}>
          This entity has no events.
        </p>
      </div>
    </section>
  );
}

function SessionBreakdown({
  handle,
  buckets,
}: {
  handle: string;
  buckets: EntityHistoryResponse['session_breakdown'];
}) {
  if (buckets.length === 0) return null;
  return (
    <section
      className="ss-card"
      data-testid="entity-session-breakdown"
    >
      <header style={{ padding: '16px 20px 0' }}>
        <div className="ss-eyebrow" style={{ marginBottom: 4 }}>
          Sessions
        </div>
        <h2
          style={{
            margin: 0,
            fontSize: 16,
            fontWeight: 600,
            letterSpacing: '-0.01em',
          }}
        >
          Appearances in {buckets.length} session
          {buckets.length === 1 ? '' : 's'}
        </h2>
      </header>
      <ul
        style={{
          listStyle: 'none',
          margin: 0,
          padding: '12px 20px 18px',
          display: 'flex',
          flexDirection: 'column',
          gap: 8,
        }}
      >
        {buckets.map((bucket) => {
          const href =
            (`/u/${encodeURIComponent(handle)}/sessions/${encodeURIComponent(bucket.session_id)}`) as Route;
          return (
            <li key={bucket.session_id}>
              <Link
                href={href}
                data-testid="entity-session-bucket"
                style={{
                  display: 'grid',
                  gridTemplateColumns: 'minmax(0, 1fr) auto auto',
                  gap: 14,
                  alignItems: 'baseline',
                  padding: '8px 12px',
                  borderRadius: 0,
                  background: 'var(--surface-1)',
                  border: '1px solid var(--border)',
                  color: 'var(--fg)',
                  textDecoration: 'none',
                }}
              >
                <span
                  className="mono"
                  style={{
                    fontSize: 12,
                    color: 'var(--fg-muted)',
                    overflow: 'hidden',
                    textOverflow: 'ellipsis',
                    whiteSpace: 'nowrap',
                  }}
                >
                  {bucket.session_id}
                </span>
                <span
                  className="mono"
                  style={{ fontSize: 11, color: 'var(--fg-dim)' }}
                  suppressHydrationWarning
                >
                  {bucket.started_at ?? '—'}
                </span>
                <span style={{ fontSize: 12 }}>
                  {bucket.event_count.toLocaleString()}
                  <span style={{ color: 'var(--fg-dim)' }}> events</span>
                </span>
              </Link>
            </li>
          );
        })}
      </ul>
    </section>
  );
}

function TimelineList({ rows }: { rows: TimelineRow[] }) {
  return (
    <section className="ss-card">
      <header style={{ padding: '16px 20px 0' }}>
        <div className="ss-eyebrow" style={{ marginBottom: 4 }}>
          Chronological
        </div>
        <h2
          style={{
            margin: 0,
            fontSize: 16,
            fontWeight: 600,
            letterSpacing: '-0.01em',
          }}
        >
          {rows.length} row{rows.length === 1 ? '' : 's'}
        </h2>
      </header>
      <ul
        style={{
          listStyle: 'none',
          margin: 0,
          padding: '12px 20px 18px',
          display: 'flex',
          flexDirection: 'column',
          gap: 8,
        }}
      >
        {rows.map((row) => (
          <TimelineRowItem key={row.key} row={row} />
        ))}
      </ul>
    </section>
  );
}

function TimelineRowItem({ row }: { row: TimelineRow }) {
  // Mirror the session-page row: surface InferredBadge when any
  // folded member is inferred, fold count when >1, raw line on hover.
  const inferredMembers = row.members.filter(
    (m) => m.metadata?.source === 'inferred',
  );
  const isInferred = inferredMembers.length > 0;
  const confidence = isInferred
    ? Math.min(...inferredMembers.map((m) => m.metadata?.confidence ?? 1))
    : (row.anchor.metadata?.confidence ?? 1);
  const ts = row.anchor.event?.timestamp ?? '';
  return (
    <li
      data-testid="timeline-row"
      style={{
        display: 'grid',
        gridTemplateColumns: 'minmax(0, 1fr) auto auto',
        gap: 12,
        alignItems: 'baseline',
        padding: '6px 10px',
        borderRadius: 0,
        background: 'var(--surface-1)',
        border: '1px solid var(--border)',
      }}
    >
      <span
        style={{
          fontSize: 13,
          color: 'var(--fg)',
          overflow: 'hidden',
          textOverflow: 'ellipsis',
          whiteSpace: 'nowrap',
        }}
        title={row.anchor.raw_line}
      >
        {rowTitleForEnvelope(row.anchor)}
        {row.count > 1 && (
          <span
            style={{
              marginLeft: 8,
              padding: '1px 6px',
              borderRadius: 0,
              background: 'var(--surface-2)',
              color: 'var(--fg-muted)',
              fontFamily: 'var(--font-mono)',
              fontSize: 11,
            }}
          >
            ×{row.count}
          </span>
        )}
      </span>
      {isInferred && <InferredBadge confidence={confidence} />}
      <span
        className="mono"
        style={{ color: 'var(--fg-dim)', fontSize: 11 }}
        suppressHydrationWarning
      >
        {ts}
      </span>
    </li>
  );
}

function LoadMoreNotice({
  handle,
  kind,
  id,
  afterCursor,
}: {
  handle: string;
  kind: string;
  id: string;
  afterCursor: string;
}) {
  const href =
    `/u/${encodeURIComponent(handle)}/entities/${encodeURIComponent(kind)}/${encodeURIComponent(id)}?after=${encodeURIComponent(afterCursor)}` as Route;
  return (
    <div
      style={{
        padding: '12px 18px',
        background: 'var(--bg-elev)',
        border: '1px solid var(--border)',
        borderRadius: 0,
        color: 'var(--fg-muted)',
        fontSize: 12,
        display: 'flex',
        gap: 12,
        alignItems: 'center',
        justifyContent: 'space-between',
      }}
    >
      <span>More events available beyond the first page.</span>
      <Link
        href={href}
        className="ss-btn ss-btn--ghost"
        data-testid="entity-history-load-more"
        style={{ textDecoration: 'none' }}
      >
        Load next page
      </Link>
    </div>
  );
}
