/**
 * Audit v2.1 §B1 — "Preview as @handle" target page.
 *
 * Server-rendered. Reads `scope` (URL-encoded JSON of ShareScope)
 * and `as` (the recipient handle, cosmetic only) from the query
 * string. Calls the simulated-preview endpoints which run the
 * OWNER's own data through the scope filter without writing audit
 * rows — so previewing leaves no trail.
 *
 * Sticky banner across the top makes it obvious this is NOT a real
 * recipient view — it's a simulation. The data, however, is the
 * real owner-side data clamped exactly as the recipient would see
 * it: tab/event-type/window/kind filters all applied server-side.
 */

import { redirect } from 'next/navigation';
import Link from 'next/link';
import {
  ApiCallError,
  previewShareSummary,
  previewShareTimeline,
  type PublicSummaryResponse,
  type PublicTimelineResponse,
} from '@/lib/api';
import { logger } from '@/lib/logger';
import { getSession } from '@/lib/session';
import { formatEventType } from '@/lib/event-types';

export const metadata = { title: "Share preview" };

interface SearchParams {
  /** URL-encoded JSON of ShareScope. */
  scope?: string;
  /** Cosmetic recipient handle for the banner. */
  as?: string;
}

type ScopeKind = 'full' | 'timeline' | 'aggregates' | 'tabs';

function parseScope(raw: string | undefined): {
  scopeJson: string | null;
  kind: ScopeKind;
  windowDays: number;
} {
  if (!raw || !raw.trim()) {
    return { scopeJson: null, kind: 'full', windowDays: 30 };
  }
  try {
    const obj = JSON.parse(raw) as {
      kind?: string;
      window_days?: number;
    };
    const kindRaw = obj.kind ?? 'full';
    const kind: ScopeKind =
      kindRaw === 'timeline' ||
      kindRaw === 'aggregates' ||
      kindRaw === 'tabs'
        ? kindRaw
        : 'full';
    const winRaw = obj.window_days;
    const windowDays =
      typeof winRaw === 'number' && winRaw > 0 ? Math.min(winRaw, 90) : 30;
    return { scopeJson: raw, kind, windowDays };
  } catch {
    return { scopeJson: null, kind: 'full', windowDays: 30 };
  }
}

export default async function SharingPreviewPage(props: {
  searchParams: Promise<SearchParams>;
}) {
  const session = await getSession();
  if (!session) redirect('/auth/login?next=/sharing');

  const params = await props.searchParams;
  const { scopeJson, kind, windowDays } = parseScope(params.scope);
  const previewingAs = (params.as ?? 'friend').trim() || 'friend';

  // Render-tier predicates mirror the server's scope_allows_* checks
  // so we can avoid hitting endpoints whose response we already know
  // will be empty — saves a round-trip on `kind=timeline` (no
  // summary) and `kind=aggregates` (no timeline) previews.
  const showSummary = kind === 'full' || kind === 'aggregates' || kind === 'tabs';
  const showTimeline = kind === 'full' || kind === 'timeline' || kind === 'tabs';

  let summary: PublicSummaryResponse | null = null;
  let timeline: PublicTimelineResponse | null = null;
  let summaryErr: string | null = null;
  let timelineErr: string | null = null;

  const calls: Array<Promise<unknown>> = [];
  if (showSummary) {
    calls.push(
      previewShareSummary(session.token, scopeJson)
        .then((r) => {
          summary = r;
        })
        .catch((e) => {
          const status = e instanceof ApiCallError ? e.status : undefined;
          summaryErr =
            e instanceof ApiCallError
              ? (e.body.error ?? `http_${e.status}`)
              : 'unknown';
          logger.error(
            { err: e, call: 'preview_share_summary', status },
            'share preview summary rejected',
          );
        }),
    );
  }
  if (showTimeline) {
    calls.push(
      previewShareTimeline(session.token, scopeJson, windowDays)
        .then((r) => {
          timeline = r;
        })
        .catch((e) => {
          const status = e instanceof ApiCallError ? e.status : undefined;
          timelineErr =
            e instanceof ApiCallError
              ? (e.body.error ?? `http_${e.status}`)
              : 'unknown';
          logger.error(
            { err: e, call: 'preview_share_timeline', status },
            'share preview timeline rejected',
          );
        }),
    );
  }
  await Promise.all(calls);

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 20 }}>
      <div
        role="status"
        style={{
          position: 'sticky',
          top: 0,
          zIndex: 10,
          padding: '10px 14px',
          background: 'color-mix(in oklab, var(--accent) 14%, var(--bg-elev))',
          border: '1px solid color-mix(in oklab, var(--accent) 40%, transparent)',
          borderRadius: 0,
          fontSize: 13,
          display: 'flex',
          justifyContent: 'space-between',
          alignItems: 'center',
          gap: 12,
          flexWrap: 'wrap',
        }}
      >
        <span>
          Previewing as <strong>@{previewingAs}</strong> &middot; this is a
          simulation — no audit row is written, no real share is granted.
          Scope kind: <code>{kind}</code>, window:{' '}
          <code>{windowDays}d</code>.
        </span>
        <Link
          href="/sharing"
          style={{ color: 'var(--fg-muted)', fontSize: 12 }}
        >
          ← Back to sharing
        </Link>
      </div>

      <header style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
        <h1 style={{ margin: 0, fontWeight: 600 }}>
          What @{previewingAs} would see
        </h1>
        <p style={{ margin: 0, color: 'var(--fg-muted)', fontSize: 14 }}>
          Your own data, rendered through the configured scope filter.
        </p>
      </header>

      {showSummary && (
        <section className="ss-card" style={{ padding: 16 }}>
          <div className="ss-eyebrow" style={{ marginBottom: 6 }}>
            Aggregates
          </div>
          <h2 style={{ margin: '0 0 12px', fontSize: 18 }}>Summary</h2>
          {summaryErr && (
            <p style={{ color: 'var(--danger)', fontSize: 13, margin: 0 }}>
              Couldn&apos;t load the preview summary ({summaryErr}). Refresh
              to retry.
            </p>
          )}
          {!summaryErr && summary && <SummaryBody summary={summary} />}
        </section>
      )}
      {!showSummary && (
        <section className="ss-card" style={{ padding: 16 }}>
          <div className="ss-eyebrow" style={{ marginBottom: 6 }}>
            Aggregates
          </div>
          <p style={{ margin: 0, color: 'var(--fg-muted)', fontSize: 13 }}>
            Hidden — scope kind <code>{kind}</code> doesn&apos;t expose
            aggregates.
          </p>
        </section>
      )}

      {showTimeline && (
        <section className="ss-card" style={{ padding: 16 }}>
          <div className="ss-eyebrow" style={{ marginBottom: 6 }}>
            Timeline · last {windowDays}d
          </div>
          <h2 style={{ margin: '0 0 12px', fontSize: 18 }}>Activity</h2>
          {timelineErr && (
            <p style={{ color: 'var(--danger)', fontSize: 13, margin: 0 }}>
              Couldn&apos;t load the preview timeline ({timelineErr}).
              Refresh to retry.
            </p>
          )}
          {!timelineErr && timeline && <TimelineBody timeline={timeline} />}
        </section>
      )}
      {!showTimeline && (
        <section className="ss-card" style={{ padding: 16 }}>
          <div className="ss-eyebrow" style={{ marginBottom: 6 }}>
            Timeline
          </div>
          <p style={{ margin: 0, color: 'var(--fg-muted)', fontSize: 13 }}>
            Hidden — scope kind <code>{kind}</code> doesn&apos;t expose the
            timeline.
          </p>
        </section>
      )}
    </div>
  );
}

function SummaryBody({ summary }: { summary: PublicSummaryResponse }) {
  const total = summary.total ?? 0;
  const types = summary.by_type ?? [];
  if (total === 0 && types.length === 0) {
    return (
      <p style={{ margin: 0, color: 'var(--fg-muted)', fontSize: 13 }}>
        No events fall inside this scope.
      </p>
    );
  }
  const sorted = [...types].sort((a, b) => (b.count ?? 0) - (a.count ?? 0));
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
      <div className="mono" style={{ fontSize: 24 }}>
        {total.toLocaleString()}
        <span
          style={{
            fontSize: 12,
            color: 'var(--fg-muted)',
            marginLeft: 8,
          }}
        >
          events total
        </span>
      </div>
      <ul
        style={{
          listStyle: 'none',
          margin: 0,
          padding: 0,
          display: 'flex',
          flexDirection: 'column',
          gap: 4,
          fontSize: 13,
        }}
      >
        {sorted.slice(0, 12).map((t) => {
          const meta = formatEventType(t.event_type);
          return (
            <li
              key={t.event_type}
              style={{
                display: 'flex',
                justifyContent: 'space-between',
                gap: 12,
                alignItems: 'baseline',
              }}
            >
              <span
                style={{
                  color: 'var(--fg)',
                  display: 'inline-flex',
                  alignItems: 'baseline',
                  gap: 8,
                  minWidth: 0,
                }}
                title={t.event_type}
              >
                <span aria-hidden="true">{meta.glyph}</span>
                <span
                  style={{
                    overflow: 'hidden',
                    textOverflow: 'ellipsis',
                    whiteSpace: 'nowrap',
                  }}
                >
                  {meta.label}
                </span>
              </span>
              <span className="mono">{(t.count ?? 0).toLocaleString()}</span>
            </li>
          );
        })}
        {sorted.length > 12 && (
          <li
            style={{
              color: 'var(--fg-dim)',
              fontStyle: 'italic',
              fontSize: 12,
            }}
          >
            +{sorted.length - 12} more types
          </li>
        )}
      </ul>
    </div>
  );
}

function TimelineBody({ timeline }: { timeline: PublicTimelineResponse }) {
  const buckets = timeline.buckets ?? [];
  const total = buckets.reduce((sum, b) => sum + (b.count ?? 0), 0);
  if (total === 0) {
    return (
      <p style={{ margin: 0, color: 'var(--fg-muted)', fontSize: 13 }}>
        No events fall inside the scope window.
      </p>
    );
  }
  const max = Math.max(1, ...buckets.map((b) => b.count ?? 0));
  return (
    <div
      style={{
        display: 'grid',
        gridTemplateColumns: `repeat(${Math.min(buckets.length, 30)}, 1fr)`,
        gap: 2,
        alignItems: 'end',
        height: 80,
      }}
      aria-label={`${total} events across ${buckets.length} buckets`}
    >
      {buckets.slice(-30).map((b) => {
        const ratio = (b.count ?? 0) / max;
        return (
          <div
            key={b.date}
            title={`${b.date}: ${b.count ?? 0}`}
            style={{
              height: `${Math.max(2, ratio * 100)}%`,
              background:
                ratio > 0
                  ? 'color-mix(in oklab, var(--accent) 60%, var(--bg-elev))'
                  : 'var(--bg-sunken)',
              borderRadius: 2,
            }}
          />
        );
      })}
    </div>
  );
}
