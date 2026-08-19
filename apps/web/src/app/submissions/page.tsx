/**
 * Submissions list — community-curated parser-rule queue.
 *
 * Mirrors the auth + error pattern of `app/dashboard/page.tsx` and
 * `app/metrics/page.tsx` verbatim: server component, session redirect
 * on missing/expired token, ApiCallError(401) → login.
 *
 * Filtering (status + mine) is fully URL-driven so the page stays a
 * pure server component — no client islands required for nav.
 *
 * Powered by GET /v1/submissions. The list response is the headline
 * page only; the four filter pills (`review`, `accepted`, `shipped`,
 * `rejected`) probe with `limit=1` to know whether the bucket is
 * non-empty, but we deliberately do NOT display a count.
 *
 * <!-- TODO: server-side count endpoint — when /v1/submissions grows a
 *      `count` mode (or the list response gains a `total`), surface
 *      real per-status counts in the filter pills. -->
 */

import Link from 'next/link';
import type { Route } from 'next';
import { redirect } from 'next/navigation';
import {
  ApiCallError,
  listSubmissions,
  type SubmissionDto,
  type SubmissionSort,
  type SubmissionStatus,
} from '@/lib/api';
import { logger } from '@/lib/logger';
import { getSession } from '@/lib/session';

export const metadata = { title: "Submissions" };

const PAGE_LIMIT = 50;

const SORT_TABS: ReadonlyArray<{ id: SubmissionSort; label: string }> = [
  { id: 'newest', label: 'Newest' },
  { id: 'oldest', label: 'Oldest' },
  { id: 'votes', label: 'Most votes' },
];

function parseSort(raw: string | undefined): SubmissionSort {
  if (raw && SORT_TABS.some((t) => t.id === raw)) return raw as SubmissionSort;
  return 'newest';
}

const FILTER_TABS: ReadonlyArray<{
  id: FilterId;
  label: string;
  /** Status param to pass to listSubmissions; `undefined` for "all". */
  status: SubmissionStatus | undefined;
}> = [
  { id: 'review', label: 'In review', status: 'review' },
  { id: 'accepted', label: 'Accepted', status: 'accepted' },
  { id: 'shipped', label: 'Shipped', status: 'shipped' },
  { id: 'rejected', label: 'Rejected', status: 'rejected' },
  { id: 'all', label: 'All', status: undefined },
];

type FilterId = 'review' | 'accepted' | 'shipped' | 'rejected' | 'all';

interface SearchParams {
  status?: string;
  mine?: string;
  sort?: string;
  offset?: string;
}

export default async function SubmissionsPage(props: {
  searchParams: Promise<SearchParams>;
}) {
  const session = await getSession();
  if (!session) redirect('/auth/login?next=/submissions');

  const params = await props.searchParams;
  const filter = parseFilter(params.status);
  const mine = params.mine === '1';
  const sort = parseSort(params.sort);
  const offset = parseOffset(params.offset);
  const activeStatus = FILTER_TABS.find((t) => t.id === filter)?.status;

  // Probes for the four "headline" status pills — limit=1 so we can
  // tell empty from non-empty without paying for a real page. Counts
  // themselves are not surfaced because the API has no total yet.
  let probeReview = false;
  let probeAccepted = false;
  let probeShipped = false;
  let probeRejected = false;

  // Settle every call independently (multi-endpoint dashboard invariant).
  // `main` is load-bearing — without the listing there is no page, so its
  // failure stays fatal. The four probes are decorative (status-pill dots);
  // a probe hiccup must never fail the page, it just greys its dot.
  const [mainResult, prResult, paResult, psResult, pjResult] =
    await Promise.allSettled([
      listSubmissions(session.token, {
        status: activeStatus,
        mine,
        sort,
        limit: PAGE_LIMIT,
        offset,
      }),
      listSubmissions(session.token, { status: 'review', limit: 1 }),
      listSubmissions(session.token, { status: 'accepted', limit: 1 }),
      listSubmissions(session.token, { status: 'shipped', limit: 1 }),
      listSubmissions(session.token, { status: 'rejected', limit: 1 }),
    ]);

  for (const [label, result] of [
    ['submissions_probe_review', prResult],
    ['submissions_probe_accepted', paResult],
    ['submissions_probe_shipped', psResult],
    ['submissions_probe_rejected', pjResult],
  ] as const) {
    if (result.status === 'rejected') {
      const status =
        result.reason instanceof ApiCallError
          ? result.reason.status
          : undefined;
      if (status === 401) redirect('/auth/login?next=/submissions');
      logger.error(
        { err: result.reason, call: label, status },
        'submissions probe rejected',
      );
    }
  }
  probeReview =
    prResult.status === 'fulfilled' && prResult.value.submissions.length > 0;
  probeAccepted =
    paResult.status === 'fulfilled' && paResult.value.submissions.length > 0;
  probeShipped =
    psResult.status === 'fulfilled' && psResult.value.submissions.length > 0;
  probeRejected =
    pjResult.status === 'fulfilled' && pjResult.value.submissions.length > 0;

  if (mainResult.status === 'rejected') {
    const status =
      mainResult.reason instanceof ApiCallError
        ? mainResult.reason.status
        : undefined;
    if (status === 401) redirect('/auth/login?next=/submissions');
    throw mainResult.reason;
  }
  const listing = mainResult.value;

  const submissions = listing.submissions;
  const showOlder = submissions.length === PAGE_LIMIT;
  const showNewer = offset > 0;

  // Derive the headline copy from the active filter so the page voice
  // stays accurate ("Pending review" / "Accepted" / etc.) without a
  // separate config table.
  const heading = headingFor(filter, mine);

  return (
    // role="main" over <main> element — avoids the global 720px column
    // (globals.css) on this full-width page (M-W9).
    <div
      role="main"
      className="ss-screen-enter"
      style={{ display: 'flex', flexDirection: 'column', gap: 20 }}
    >
      <header>
        <div className="ss-eyebrow" style={{ marginBottom: 8 }}>
          Submissions · community-curated parser rules
        </div>
        <div
          style={{
            display: 'flex',
            alignItems: 'flex-start',
            justifyContent: 'space-between',
            gap: 20,
            flexWrap: 'wrap',
          }}
        >
          <div style={{ flex: '1 1 320px' }}>
            <h1
              style={{
                margin: 0,
                fontSize: 32,
                fontWeight: 600,
                letterSpacing: '-0.02em',
              }}
            >
              {heading}
            </h1>
            <p
              style={{
                margin: '6px 0 0',
                color: 'var(--fg-muted)',
                fontSize: 14,
                maxWidth: 640,
              }}
            >
              When the desktop client spots a log line it doesn&apos;t
              recognise, you can submit it for review. The useful ones are
              published here for the community to vote on, and accepted
              patterns ship in the next parser update.
            </p>
          </div>
          <Link
            href={'/submissions/new' as Route}
            className="ss-btn ss-btn--ghost"
          >
            Submit a new pattern
          </Link>
        </div>
      </header>

      {/* Filter tabs + Mine toggle. */}
      <nav
        aria-label="Submission filters"
        style={{ display: 'flex', flexWrap: 'wrap', gap: 6 }}
      >
        {FILTER_TABS.map((t) => {
          const active = t.id === filter;
          const probed =
            t.id === 'review'
              ? probeReview
              : t.id === 'accepted'
                ? probeAccepted
                : t.id === 'shipped'
                  ? probeShipped
                  : t.id === 'rejected'
                    ? probeRejected
                    : null;
          return (
            <Link
              key={t.id}
              href={buildHref({ filter: t.id, mine, sort })}
              data-active={active ? 'true' : undefined}
              style={{
                background: active ? 'var(--bg-elev)' : 'transparent',
                border: '1px solid',
                borderColor: active ? 'var(--border-strong)' : 'transparent',
                color: active ? 'var(--fg)' : 'var(--fg-muted)',
                padding: '8px 14px',
                borderRadius: 'var(--r-pill)',
                fontSize: 13,
                textDecoration: 'none',
                display: 'inline-flex',
                alignItems: 'center',
                gap: 8,
              }}
            >
              <span>{t.label}</span>
              {probed !== null && (
                <span
                  className="mono"
                  aria-hidden="true"
                  style={{
                    width: 6,
                    height: 6,
                    borderRadius: 999,
                    background: probed
                      ? 'var(--accent)'
                      : 'var(--border-strong)',
                  }}
                />
              )}
            </Link>
          );
        })}
        <span
          aria-hidden="true"
          style={{ width: 1, background: 'var(--border)', margin: '0 4px' }}
        />
        <Link
          href={buildHref({ filter, mine: !mine, sort })}
          data-active={mine ? 'true' : undefined}
          style={{
            background: mine ? 'var(--accent-soft)' : 'transparent',
            border: '1px solid',
            borderColor: mine ? 'var(--accent)' : 'var(--border)',
            color: mine ? 'var(--accent)' : 'var(--fg-muted)',
            padding: '8px 14px',
            borderRadius: 'var(--r-pill)',
            fontSize: 13,
            textDecoration: 'none',
          }}
        >
          {mine ? '✓ Mine' : 'Mine'}
        </Link>
      </nav>

      {/* Sort control — newest-first default, with oldest + most-votes.
          Changing sort resets to the first page (offset omitted). */}
      <nav
        aria-label="Sort submissions"
        style={{ display: 'flex', alignItems: 'center', flexWrap: 'wrap', gap: 6 }}
      >
        <span
          style={{
            fontSize: 11,
            color: 'var(--fg-dim)',
            letterSpacing: '0.04em',
            marginRight: 2,
          }}
        >
          Sort
        </span>
        {SORT_TABS.map((t) => {
          const active = t.id === sort;
          return (
            <Link
              key={t.id}
              href={buildHref({ filter, mine, sort: t.id })}
              data-active={active ? 'true' : undefined}
              style={{
                background: active ? 'var(--bg-elev)' : 'transparent',
                border: '1px solid',
                borderColor: active ? 'var(--border-strong)' : 'transparent',
                color: active ? 'var(--fg)' : 'var(--fg-muted)',
                padding: '6px 12px',
                borderRadius: 'var(--r-pill)',
                fontSize: 12,
                textDecoration: 'none',
              }}
            >
              {t.label}
            </Link>
          );
        })}
      </nav>

      {submissions.length === 0 ? (
        <section className="ss-card" style={{ padding: '40px 24px' }}>
          <div className="ss-eyebrow" style={{ marginBottom: 6 }}>
            Empty queue
          </div>
          <h2
            style={{
              margin: 0,
              fontSize: 17,
              fontWeight: 600,
              letterSpacing: '-0.01em',
            }}
          >
            Scope is clear. No submissions match this filter.
          </h2>
        </section>
      ) : (
        <ul
          style={{
            listStyle: 'none',
            margin: 0,
            padding: 0,
            display: 'flex',
            flexDirection: 'column',
            gap: 10,
          }}
        >
          {submissions.map((s) => (
            <li key={s.id}>
              <SubmissionRow
                submission={s}
                viewerUserId={session.userId}
                viewerHandle={session.claimedHandle}
              />
            </li>
          ))}
        </ul>
      )}

      {(showOlder || showNewer) && (
        <nav
          aria-label="Submission pagination"
          style={{
            display: 'flex',
            justifyContent: 'space-between',
            gap: 12,
            paddingTop: 8,
            fontSize: 13,
          }}
        >
          {showNewer ? (
            <Link
              href={buildHref({
                filter,
                mine,
                sort,
                offset: Math.max(0, offset - PAGE_LIMIT),
              })}
              className="ss-btn ss-btn--ghost"
            >
              ← Newer
            </Link>
          ) : (
            <span style={{ color: 'var(--fg-dim)' }}>← Newer</span>
          )}
          {showOlder ? (
            <Link
              href={buildHref({
                filter,
                mine,
                sort,
                offset: offset + PAGE_LIMIT,
              })}
              className="ss-btn ss-btn--ghost"
            >
              Older →
            </Link>
          ) : (
            <span style={{ color: 'var(--fg-dim)' }}>Older →</span>
          )}
        </nav>
      )}
    </div>
  );
}

// -- Row ------------------------------------------------------------

function SubmissionRow({
  submission,
  viewerUserId,
  viewerHandle,
}: {
  submission: SubmissionDto;
  viewerUserId: string;
  viewerHandle: string;
}) {
  // Derive `mine` client-side: prefer ID match (canonical), fall back
  // to handle match in case the cookie pre-dates the userId field
  // (unlikely but cheap).
  const isMine =
    submission.submitter_id === viewerUserId ||
    submission.submitter_handle === viewerHandle;

  // Row redesign per v1 audit §05: "status as a left-edge accent
  // strip, pattern inline with metrics, flag as soft text, single
  // clear CTA on the right." The previous row had four competing
  // visual zones (vote tile, badges row, body, footer); this version
  // routes status through a 3px coloured strip, drops the StatusPill
  // chip, demotes flag count to muted inline text, and adds an
  // explicit "Open →" affordance on the right.
  const statusAccent = accentForStatus(submission.status);

  return (
    <Link
      href={
        `/submissions/${encodeURIComponent(submission.id)}` as Route
      }
      className="ss-card"
      style={{
        display: 'grid',
        gridTemplateColumns: '3px auto 1fr auto',
        gap: 14,
        alignItems: 'center',
        padding: '14px 16px 14px 0',
        textDecoration: 'none',
        color: 'inherit',
      }}
    >
      {/* Left-edge status strip — replaces the StatusPill chip. */}
      <span
        aria-hidden="true"
        style={{
          alignSelf: 'stretch',
          background: statusAccent,
          borderRadius: '0 2px 2px 0',
        }}
      />

      {/* Vote tile — kept (the row's primary action affordance) but
          softened so it doesn't compete with the rest of the card. */}
      <div
        style={{
          display: 'flex',
          flexDirection: 'column',
          alignItems: 'center',
          gap: 2,
          color: submission.viewer_voted
            ? 'var(--accent)'
            : 'var(--fg-muted)',
          padding: '4px 10px',
          minWidth: 48,
        }}
        aria-label={`${submission.vote_count} ${submission.vote_count === 1 ? 'vote' : 'votes'}`}
      >
        <span style={{ fontSize: 13, letterSpacing: '0.05em' }}>
          {submission.viewer_voted ? '▲' : '△'}
        </span>
        <span
          className="mono"
          style={{ fontSize: 13, fontWeight: 600 }}
        >
          {submission.vote_count.toLocaleString()}
        </span>
      </div>

      <div style={{ minWidth: 0 }}>
        <div
          className="mono"
          style={{
            fontSize: 13,
            color: 'var(--accent)',
            marginBottom: 2,
            overflow: 'hidden',
            textOverflow: 'ellipsis',
            whiteSpace: 'nowrap',
          }}
        >
          {submission.proposed_label}
          {isMine && (
            <span
              style={{
                marginLeft: 8,
                fontSize: 10,
                color: 'var(--fg-muted)',
                fontFamily: 'inherit',
                letterSpacing: '0.1em',
                textTransform: 'uppercase',
              }}
            >
              · mine
            </span>
          )}
        </div>
        <div
          style={{
            fontSize: 13,
            color: 'var(--fg-muted)',
            marginBottom: 4,
            overflow: 'hidden',
            textOverflow: 'ellipsis',
            display: '-webkit-box',
            WebkitLineClamp: 2,
            WebkitBoxOrient: 'vertical',
          }}
        >
          {submission.description}
        </div>
        {/* Pattern inline with metrics — the row's metadata line. */}
        <div
          style={{
            display: 'flex',
            alignItems: 'baseline',
            gap: 10,
            fontSize: 11,
            color: 'var(--fg-dim)',
            flexWrap: 'wrap',
          }}
        >
          <code
            className="mono"
            style={{
              color: 'var(--fg-muted)',
              overflow: 'hidden',
              textOverflow: 'ellipsis',
              whiteSpace: 'nowrap',
              maxWidth: '40ch',
            }}
            title={submission.pattern}
          >
            {submission.pattern}
          </code>
          <span aria-hidden="true">·</span>
          <span className="mono">{shortId(submission.id)}</span>
          <span aria-hidden="true">·</span>
          <span>
            by{' '}
            <span className="mono" style={{ color: 'var(--fg-muted)' }}>
              {submission.submitter_handle}
            </span>
          </span>
          <span aria-hidden="true">·</span>
          <span>{formatRelativeTime(submission.created_at)}</span>
          {submission.flag_count > 0 && (
            <>
              <span aria-hidden="true">·</span>
              <span style={{ color: 'var(--warn)' }}>
                {submission.flag_count}{' '}
                {submission.flag_count === 1 ? 'flag' : 'flags'}
              </span>
            </>
          )}
        </div>
      </div>

      {/* Single clear CTA on the right (replaces the implicit
          whole-row link affordance with an explicit one). */}
      <span
        aria-hidden="true"
        className="mono"
        style={{
          color: 'var(--fg-dim)',
          fontSize: 11,
          letterSpacing: '0.08em',
          textTransform: 'uppercase',
          padding: '0 4px',
        }}
      >
        Open →
      </span>
    </Link>
  );
}

/// Maps each submission status to the row's left-edge accent colour.
/// Keeps the lifecycle visible at a glance without needing the chip
/// the previous row had — the same colour signals the StatusPill used,
/// in a less competing form-factor.
///
/// Parameter is widened to `string` because `SubmissionDto.status`
/// comes from the OpenAPI schema where the field is a free-form
/// string, not the narrowed `SubmissionStatus` union we use in
/// request params. The default branch handles forward-compat with
/// any new lifecycle state the server starts emitting before the
/// client schema regenerates.
function accentForStatus(status: string): string {
  switch (status) {
    case 'review':
      return 'var(--warn)';
    case 'accepted':
      return 'var(--ok)';
    case 'shipped':
      return 'var(--accent)';
    case 'rejected':
      return 'var(--danger)';
    case 'flagged':
      return 'var(--danger)';
    case 'withdrawn':
      return 'var(--fg-dim)';
    default:
      return 'var(--border-strong)';
  }
}

// -- Helpers --------------------------------------------------------

function parseFilter(raw: string | undefined): FilterId {
  if (raw === undefined) return 'review';
  if (FILTER_TABS.some((t) => t.id === raw)) return raw as FilterId;
  return 'review';
}

function parseOffset(raw: string | undefined): number {
  if (!raw) return 0;
  const n = Number(raw);
  if (!Number.isFinite(n) || n < 0) return 0;
  return Math.floor(n);
}

function buildHref(opts: {
  filter: FilterId;
  mine: boolean;
  sort: SubmissionSort;
  offset?: number;
}): Route {
  const qs = new URLSearchParams();
  // `review` is the default — omit so the bare URL is canonical.
  if (opts.filter !== 'review') qs.set('status', opts.filter);
  if (opts.mine) qs.set('mine', '1');
  // `newest` is the default — omit to keep the bare URL canonical.
  if (opts.sort !== 'newest') qs.set('sort', opts.sort);
  if (opts.offset !== undefined && opts.offset > 0) {
    qs.set('offset', String(opts.offset));
  }
  const suffix = qs.toString();
  return (suffix ? `/submissions?${suffix}` : '/submissions') as Route;
}

function headingFor(filter: FilterId, mine: boolean): string {
  if (mine && filter === 'all') return 'Your submissions';
  if (mine) return `Your submissions · ${labelFor(filter)}`;
  switch (filter) {
    case 'review':
      return 'Pending review';
    case 'accepted':
      return 'Accepted';
    case 'shipped':
      return 'Shipped';
    case 'rejected':
      return 'Rejected';
    case 'all':
      return 'All submissions';
  }
}

function labelFor(filter: FilterId): string {
  switch (filter) {
    case 'review':
      return 'in review';
    case 'accepted':
      return 'accepted';
    case 'shipped':
      return 'shipped';
    case 'rejected':
      return 'rejected';
    case 'all':
      return 'all';
  }
}

function shortId(id: string): string {
  // The server emits UUIDs for submissions. The design mocks "SUB-2418";
  // we mimic that aesthetic with a short prefix of the real id so the
  // copy still looks like a submission ticket reference.
  if (id.length <= 8) return id;
  return `SUB-${id.slice(0, 8)}`;
}

function formatRelativeTime(iso: string): string {
  const ts = new Date(iso).getTime();
  if (Number.isNaN(ts)) return iso;
  const diffMs = Date.now() - ts;
  if (diffMs < 60_000) return 'just now';
  if (diffMs < 3_600_000) return `${Math.floor(diffMs / 60_000)}m ago`;
  if (diffMs < 86_400_000) return `${Math.floor(diffMs / 3_600_000)}h ago`;
  if (diffMs < 7 * 86_400_000) {
    return `${Math.floor(diffMs / 86_400_000)}d ago`;
  }
  return new Date(iso).toLocaleDateString(undefined, {
    month: 'short',
    day: 'numeric',
  });
}
