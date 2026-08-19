/**
 * Cross-session entity rollup index for a user.
 *
 * Server component — fetches `/v1/users/{handle}/entities` with the
 * visitor's bearer token and renders a filterable grid of entity
 * cards. Each card links to `/u/{handle}/entities/{kind}/{id}` for
 * the full per-entity event history.
 *
 * Access control is server-side. The fetch will 401 if the cookie is
 * stale or 403 if the visitor lacks `share_event_timeline`; both
 * collapse to the same "not available" render so we never leak
 * whether the user has entity data.
 *
 * Kind filter — `?kind=vehicle` etc. — runs client-side over the
 * already-fetched list. Filter chip buttons re-render the page with
 * the URL param set so the view stays shareable.
 */

import Link from 'next/link';
import type { Route } from 'next';
import {
  ApiCallError,
  getUserEntities,
  type EntitiesListResponse,
  type EntitySummary,
} from '@/lib/api';
import { logger } from '@/lib/logger';
import { getSession } from '@/lib/session';
import {
  ENTITY_KINDS,
  labelForEntityKind,
} from '@/lib/timeline-metadata';

export const metadata = { title: "Entities" };

interface PageProps {
  params: Promise<{ handle: string }>;
  searchParams: Promise<{ kind?: string; after?: string }>;
}

const KIND_FILTERS: ReadonlyArray<string> = ['all', ...ENTITY_KINDS];

function parseKindFilter(raw: string | undefined): string {
  if (raw == null || raw === 'all') return 'all';
  return KIND_FILTERS.includes(raw) ? raw : 'all';
}

/**
 * Discriminator used to distinguish the success and forbidden
 * responses from `loadEntities`. We can't reuse a `kind` field on the
 * error case because `EntitySummary` itself has a `kind` field —
 * TypeScript's narrowing would collapse both branches. Matches the
 * `ForbiddenSentinel` shape used by the per-entity history page.
 */
interface ForbiddenSentinel {
  forbidden: true;
}

async function loadEntities(
  bearer: string,
  handle: string,
  after: string | undefined,
): Promise<EntitiesListResponse | ForbiddenSentinel> {
  try {
    return await getUserEntities(bearer, handle, { after });
  } catch (e) {
    if (
      e instanceof ApiCallError &&
      (e.status === 401 || e.status === 403 || e.status === 404)
    ) {
      return { forbidden: true };
    }
    logger.error({ err: e }, 'entities list fetch failed');
    return { forbidden: true };
  }
}

export default async function EntitiesIndexPage(props: PageProps) {
  const { handle } = await props.params;
  const search = await props.searchParams;
  const activeKind = parseKindFilter(search.kind);

  const session = await getSession();
  if (!session) {
    return <EntitiesUnavailable handle={handle} />;
  }

  const result = await loadEntities(session.token, handle, search.after);
  if ('forbidden' in result) {
    return <EntitiesUnavailable handle={handle} />;
  }

  const filtered =
    activeKind === 'all'
      ? result.entities
      : result.entities.filter((e) => e.kind === activeKind);

  return (
    <div
      className="ss-screen-enter"
      style={{ display: 'flex', flexDirection: 'column', gap: 20 }}
    >
      <header>
        <div className="ss-eyebrow" style={{ marginBottom: 8 }}>
          Entities
        </div>
        <h1
          style={{
            margin: 0,
            fontSize: 32,
            fontWeight: 600,
            letterSpacing: '-0.02em',
          }}
        >
          <span className="mono">{handle}</span>
          <span style={{ color: 'var(--fg-dim)' }}>{' / '}</span>
          <span style={{ color: 'var(--fg-muted)', fontSize: 24 }}>
            Entities
          </span>
        </h1>
        <p
          style={{
            margin: '8px 0 0',
            color: 'var(--fg-muted)',
            fontSize: 14,
            lineHeight: 1.5,
          }}
        >
          Everything that ever happened to a particular ship, player,
          location, or item — aggregated across all sessions.
          {' '}
          <span style={{ color: 'var(--fg-dim)' }}>
            {result.entities.length.toLocaleString()} entit
            {result.entities.length === 1 ? 'y' : 'ies'} tracked
            {result.next_after ? ' (more available)' : ''}.
          </span>
        </p>
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
            href={(`/u/${encodeURIComponent(handle)}`) as Route}
            className="ss-btn ss-btn--ghost"
            style={{ textDecoration: 'none' }}
          >
            ← Back to profile
          </Link>
        </div>
      </header>

      <KindFilterBar handle={handle} active={activeKind} />

      {result.entities.length === 0 ? (
        <EntitiesEmpty />
      ) : filtered.length === 0 ? (
        <EntitiesEmptyForFilter kind={activeKind} />
      ) : (
        <ul
          data-testid="entities-grid"
          style={{
            listStyle: 'none',
            margin: 0,
            padding: 0,
            display: 'grid',
            gridTemplateColumns: 'repeat(auto-fill, minmax(260px, 1fr))',
            gap: 14,
          }}
        >
          {filtered.map((entity) => (
            <li key={`${entity.kind}:${entity.id}`}>
              <EntityCard handle={handle} entity={entity} />
            </li>
          ))}
        </ul>
      )}

      {result.next_after && (
        <LoadMoreNotice handle={handle} afterCursor={result.next_after} />
      )}
    </div>
  );
}

function EntitiesUnavailable({ handle }: { handle: string }) {
  return (
    <div
      className="ss-screen-enter"
      style={{ display: 'flex', flexDirection: 'column', gap: 20 }}
      data-testid="entities-forbidden"
    >
      <header>
        <div className="ss-eyebrow" style={{ marginBottom: 8 }}>
          Entities
        </div>
        <h1
          style={{
            margin: 0,
            fontSize: 28,
            fontWeight: 600,
            letterSpacing: '-0.02em',
          }}
        >
          Entities not available
        </h1>
        <p
          style={{
            margin: '6px 0 0',
            color: 'var(--fg-muted)',
            fontSize: 14,
          }}
        >
          This player hasn&apos;t shared their event history with you.
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

function EntitiesEmpty() {
  return (
    <section className="ss-card" data-testid="entities-empty-state">
      <div style={{ padding: '32px 24px', textAlign: 'center' }}>
        <p style={{ margin: 0, color: 'var(--fg-muted)', fontSize: 14 }}>
          No entities tracked yet.
        </p>
      </div>
    </section>
  );
}

function EntitiesEmptyForFilter({ kind }: { kind: string }) {
  return (
    <section className="ss-card" data-testid="entities-filter-empty">
      <div style={{ padding: '24px', textAlign: 'center' }}>
        <p style={{ margin: 0, color: 'var(--fg-muted)', fontSize: 13 }}>
          No <span className="mono">{kind}</span> entities recorded.
        </p>
      </div>
    </section>
  );
}

function KindFilterBar({
  handle,
  active,
}: {
  handle: string;
  active: string;
}) {
  // Client-side filter via URL param so the chosen view stays
  // shareable. Each chip is a server-resolved link, NOT a button —
  // keeps the page a pure server component and means the back/forward
  // browser navigation works correctly.
  return (
    <div
      role="group"
      aria-label="Filter by entity kind"
      data-testid="entities-filter-bar"
      style={{
        display: 'flex',
        gap: 6,
        flexWrap: 'wrap',
        padding: 4,
        background: 'var(--bg-elev)',
        border: '1px solid var(--border)',
        borderRadius: 'var(--r-sm)',
      }}
    >
      {KIND_FILTERS.map((kind) => {
        const isActive = kind === active;
        const params = kind === 'all' ? '' : `?kind=${kind}`;
        const href = (`/u/${encodeURIComponent(handle)}/entities${params}`) as Route;
        const label = kind === 'all' ? 'All' : labelForEntityKind(kind);
        return (
          <Link
            key={kind}
            href={href}
            data-testid={`entities-filter-${kind}`}
            data-active={isActive ? 'true' : 'false'}
            aria-current={isActive ? 'page' : undefined}
            className={
              isActive ? 'ss-btn ss-btn--primary' : 'ss-btn ss-btn--ghost'
            }
            style={{ textDecoration: 'none', fontSize: 12 }}
          >
            {label}
          </Link>
        );
      })}
    </div>
  );
}

function EntityCard({
  handle,
  entity,
}: {
  handle: string;
  entity: EntitySummary;
}) {
  const href =
    (`/u/${encodeURIComponent(handle)}/entities/${encodeURIComponent(entity.kind)}/${encodeURIComponent(entity.id)}`) as Route;
  const firstSeen = entity.first_seen ?? null;
  const lastSeen = entity.last_seen ?? null;
  return (
    <Link
      href={href}
      data-testid="entity-card"
      data-kind={entity.kind}
      data-entity-id={entity.id}
      style={{
        display: 'flex',
        flexDirection: 'column',
        gap: 8,
        padding: '16px 18px',
        background: 'var(--surface-1)',
        border: '1px solid var(--border)',
        borderRadius: 'var(--r-sm)',
        textDecoration: 'none',
        color: 'var(--fg)',
        minHeight: 120,
      }}
    >
      <div
        style={{
          display: 'flex',
          gap: 8,
          alignItems: 'center',
          justifyContent: 'space-between',
        }}
      >
        <span className="ss-eyebrow">{labelForEntityKind(entity.kind)}</span>
      </div>
      <div
        title={entity.display_name}
        style={{
          fontSize: 16,
          fontWeight: 600,
          letterSpacing: '-0.01em',
          overflow: 'hidden',
          textOverflow: 'ellipsis',
          whiteSpace: 'nowrap',
        }}
      >
        {entity.display_name}
      </div>
      <div
        className="mono"
        style={{
          fontSize: 11,
          color: 'var(--fg-dim)',
          overflow: 'hidden',
          textOverflow: 'ellipsis',
          whiteSpace: 'nowrap',
        }}
      >
        {entity.id}
      </div>
      <div
        style={{
          marginTop: 'auto',
          display: 'flex',
          gap: 14,
          alignItems: 'baseline',
          flexWrap: 'wrap',
          fontSize: 12,
          color: 'var(--fg-muted)',
        }}
      >
        <span>
          {entity.event_count.toLocaleString()} event
          {entity.event_count === 1 ? '' : 's'}
        </span>
        <span>
          {entity.session_count.toLocaleString()} session
          {entity.session_count === 1 ? '' : 's'}
        </span>
      </div>
      {(firstSeen || lastSeen) && (
        <div
          className="mono"
          style={{ fontSize: 10, color: 'var(--fg-dim)' }}
          suppressHydrationWarning
        >
          {firstSeen ?? '?'} → {lastSeen ?? '?'}
        </div>
      )}
    </Link>
  );
}

function LoadMoreNotice({
  handle,
  afterCursor,
}: {
  handle: string;
  afterCursor: string;
}) {
  const href = `/u/${encodeURIComponent(handle)}/entities?after=${encodeURIComponent(afterCursor)}` as Route;
  return (
    <div
      style={{
        padding: '12px 18px',
        background: 'var(--bg-elev)',
        border: '1px solid var(--border)',
        borderRadius: 'var(--r-sm)',
        color: 'var(--fg-muted)',
        fontSize: 12,
        display: 'flex',
        gap: 12,
        alignItems: 'center',
        justifyContent: 'space-between',
      }}
    >
      <span>More entities available beyond the first page.</span>
      <Link
        href={href}
        className="ss-btn ss-btn--ghost"
        data-testid="entities-load-more"
        style={{ textDecoration: 'none' }}
      >
        Load next page
      </Link>
    </div>
  );
}
