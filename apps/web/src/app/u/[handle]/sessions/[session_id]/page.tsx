/**
 * Per-event timeline for one session.
 *
 * Server component — fetches `/v1/users/{handle}/sessions/{session_id}/events`
 * with the visitor's bearer token and renders the entity-grouped
 * timeline using the Phase 5 helpers in `lib/timeline-metadata.ts`
 * (`groupEventsForTimeline`, `foldAdjacentSameKey`, `rowTitleForEnvelope`)
 * plus the `InferredBadge` for `metadata.source === 'inferred'` rows.
 *
 * Access control is server-side. The fetch will 401 if the cookie is
 * stale or 403 if the visitor lacks `share_event_timeline`; both
 * collapse to the same "Session not available" render so we never
 * leak whether the session exists.
 *
 * View toggle — by-entity (default) or chronological — lives in the
 * URL search params as `?view=chronological`. Persisting the view in
 * the URL makes the page shareable as a direct link.
 *
 * Submission UI is intentionally absent: per the design spec, parser
 * submission is a tray-only affordance.
 */

import Link from 'next/link';
import type { Route } from 'next';
import {
  ApiCallError,
  getSessionEvents,
  type SessionEventsResponse,
} from '@/lib/api';
import { logger } from '@/lib/logger';
import { getSession } from '@/lib/session';
import { InferredBadge } from '@/components/InferredBadge';
import {
  foldAdjacentSameKey,
  groupEventsForTimeline,
  rowTitleForEnvelope,
  type EntitySection,
  type TimelineRow,
} from '@/lib/timeline-metadata';
import type { EventEnvelope } from 'api-client-ts';

export const metadata = { title: "Session" };

type ViewMode = 'entity' | 'chronological';

interface PageProps {
  params: Promise<{ handle: string; session_id: string }>;
  searchParams: Promise<{ view?: string }>;
}

function parseView(raw: string | undefined): ViewMode {
  // Default to entity grouping — the by-entity layout was the goal of
  // the audit-v2 redesign. Any other value (including missing) falls
  // back to the default; we deliberately don't 404 on a typo.
  return raw === 'chronological' ? 'chronological' : 'entity';
}

async function loadEvents(
  bearer: string,
  handle: string,
  sessionId: string,
): Promise<SessionEventsResponse | { kind: 'forbidden' }> {
  try {
    return await getSessionEvents(bearer, handle, sessionId);
  } catch (e) {
    if (
      e instanceof ApiCallError &&
      (e.status === 401 || e.status === 403 || e.status === 404)
    ) {
      return { kind: 'forbidden' };
    }
    logger.error({ err: e }, 'session events fetch failed');
    return { kind: 'forbidden' };
  }
}

export default async function SessionTimelinePage(props: PageProps) {
  const { handle, session_id } = await props.params;
  const search = await props.searchParams;
  const view = parseView(search.view);

  const session = await getSession();
  if (!session) {
    return <SessionUnavailable handle={handle} />;
  }

  const result = await loadEvents(session.token, handle, session_id);
  if ('kind' in result) {
    return <SessionUnavailable handle={handle} />;
  }

  const { events, next_after, session_id: confirmedId } = result;

  return (
    <div
      className="ss-screen-enter"
      style={{ display: 'flex', flexDirection: 'column', gap: 20 }}
    >
      <header>
        <div className="ss-eyebrow" style={{ marginBottom: 8 }}>
          Session timeline
        </div>
        <h1
          style={{
            margin: 0,
            fontSize: 28,
            fontWeight: 600,
            letterSpacing: '-0.02em',
          }}
        >
          <span className="mono">{handle}</span>
          <span style={{ color: 'var(--fg-dim)' }}>{' / '}</span>
          <span className="mono" style={{ color: 'var(--fg-muted)' }}>
            {confirmedId}
          </span>
        </h1>
        <div
          style={{
            marginTop: 10,
            display: 'flex',
            gap: 10,
            alignItems: 'center',
            flexWrap: 'wrap',
          }}
        >
          <Link
            href={(`/u/${encodeURIComponent(handle)}`) as Route}
            className="ss-btn ss-btn--ghost"
            style={{ textDecoration: 'none' }}
          >
            ← Back to profile
          </Link>
          <ViewToggle handle={handle} sessionId={confirmedId} active={view} />
          <span style={{ color: 'var(--fg-dim)', fontSize: 13 }}>
            {events.length.toLocaleString()} event
            {events.length === 1 ? '' : 's'}
            {next_after ? ' (more available)' : ''}
          </span>
        </div>
      </header>

      {events.length === 0 ? (
        <EmptySession />
      ) : view === 'entity' ? (
        <EntityView handle={handle} events={events} />
      ) : (
        <ChronologicalView events={events} />
      )}

      {next_after && (
        <LoadMoreNotice
          handle={handle}
          sessionId={confirmedId}
          afterCursor={next_after}
        />
      )}
    </div>
  );
}

function SessionUnavailable({ handle }: { handle: string }) {
  return (
    <div
      className="ss-screen-enter"
      style={{ display: 'flex', flexDirection: 'column', gap: 20 }}
    >
      <header>
        <div className="ss-eyebrow" style={{ marginBottom: 8 }}>
          Session timeline
        </div>
        <h1
          style={{
            margin: 0,
            fontSize: 28,
            fontWeight: 600,
            letterSpacing: '-0.02em',
          }}
        >
          Session not available
        </h1>
        <p
          style={{
            margin: '6px 0 0',
            color: 'var(--fg-muted)',
            fontSize: 14,
          }}
        >
          This session is either private or not shared with you.
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

function EmptySession() {
  return (
    <section className="ss-card">
      <div style={{ padding: '24px', textAlign: 'center' }}>
        <p style={{ margin: 0, color: 'var(--fg-muted)', fontSize: 14 }}>
          Session has no events.
        </p>
      </div>
    </section>
  );
}

function ViewToggle({
  handle,
  sessionId,
  active,
}: {
  handle: string;
  sessionId: string;
  active: ViewMode;
}) {
  const base = `/u/${encodeURIComponent(handle)}/sessions/${encodeURIComponent(sessionId)}`;
  return (
    <div
      role="group"
      aria-label="View mode"
      style={{
        display: 'inline-flex',
        gap: 4,
        padding: 4,
        background: 'var(--bg-elev)',
        border: '1px solid var(--border)',
        borderRadius: 0,
      }}
    >
      <Link
        href={(`${base}`) as Route}
        className={
          active === 'entity'
            ? 'ss-btn ss-btn--primary'
            : 'ss-btn ss-btn--ghost'
        }
        style={{ textDecoration: 'none', fontSize: 12 }}
        aria-current={active === 'entity' ? 'page' : undefined}
      >
        By entity
      </Link>
      <Link
        href={(`${base}?view=chronological`) as Route}
        className={
          active === 'chronological'
            ? 'ss-btn ss-btn--primary'
            : 'ss-btn ss-btn--ghost'
        }
        style={{ textDecoration: 'none', fontSize: 12 }}
        aria-current={active === 'chronological' ? 'page' : undefined}
      >
        Chronological
      </Link>
    </div>
  );
}

function EntityView({
  handle,
  events,
}: {
  handle: string;
  events: EventEnvelope[];
}) {
  const sections = groupEventsForTimeline(events);
  // Some envelopes may lack metadata. Fall through to a chronological
  // tail when nothing groups so the page never shows an empty state
  // alongside a non-empty event list.
  if (sections.length === 0) {
    return <ChronologicalView events={events} />;
  }
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 16 }}>
      {sections.map((section) => (
        <EntitySectionCard
          key={`${section.entity.kind}:${section.entity.id}`}
          handle={handle}
          section={section}
        />
      ))}
    </div>
  );
}

function EntitySectionCard({
  handle,
  section,
}: {
  handle: string;
  section: EntitySection;
}) {
  // Cross-link the section header to the entity's cross-session
  // rollup page. The whole header (eyebrow + display name) becomes
  // the link target so the affordance is obvious without adding a
  // separate "View all" button on each card.
  const href =
    (`/u/${encodeURIComponent(handle)}/entities/${encodeURIComponent(section.entity.kind)}/${encodeURIComponent(section.entity.id)}`) as Route;
  return (
    <section className="ss-card" data-testid="entity-section">
      <header style={{ padding: '16px 20px 0' }}>
        <Link
          href={href}
          data-testid="entity-section-link"
          aria-label={`View all events for ${section.entity.display_name} across sessions`}
          style={{
            display: 'block',
            textDecoration: 'none',
            color: 'var(--fg)',
          }}
        >
          <div className="ss-eyebrow" style={{ marginBottom: 4 }}>
            {section.entity.kind}
          </div>
          <h2
            style={{
              margin: 0,
              fontSize: 16,
              fontWeight: 600,
              letterSpacing: '-0.01em',
            }}
          >
            {section.entity.display_name}
            <span
              aria-hidden="true"
              style={{
                marginLeft: 8,
                color: 'var(--accent)',
                fontSize: 13,
              }}
            >
              →
            </span>
          </h2>
        </Link>
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
        {section.rows.map((row) => (
          <TimelineRowItem key={row.key} row={row} />
        ))}
      </ul>
    </section>
  );
}

function ChronologicalView({ events }: { events: EventEnvelope[] }) {
  const rows = foldAdjacentSameKey(events);
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
  // A folded row may contain a mix of observed + inferred members
  // (same group_key, different `source`). Surface the badge when ANY
  // member is inferred so the row signals "this collapses an inferred
  // event"; the per-event source is still inspectable on drill-in.
  // The displayed confidence is the lowest among inferred members so
  // the badge errs on the side of caution.
  const inferredMembers = row.members.filter(
    (m) => m.metadata?.source === 'inferred',
  );
  const isInferred = inferredMembers.length > 0;
  const confidence = isInferred
    ? Math.min(
        ...inferredMembers.map((m) => m.metadata?.confidence ?? 1),
      )
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
  sessionId,
  afterCursor,
}: {
  handle: string;
  sessionId: string;
  afterCursor: string;
}) {
  // The page is a server component; "Load more" navigates to the same
  // route with the cursor in the URL so the server can fetch the next
  // page on render. Persisting the cursor in the URL keeps the page
  // shareable.
  const href = `/u/${encodeURIComponent(handle)}/sessions/${encodeURIComponent(sessionId)}?after=${encodeURIComponent(afterCursor)}` as Route;
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
      <Link href={href} className="ss-btn ss-btn--ghost" style={{ textDecoration: 'none' }}>
        Load next page
      </Link>
    </div>
  );
}
