/**
 * Admin moderation queue.
 *
 * Mirrors the user-facing `app/submissions/page.tsx` pattern: server
 * component, URL-driven filter, Promise.all probe for filter chips,
 * 50-per-page offset pagination. The admin queue intentionally
 * excludes accepted/shipped/rejected — those are not actionable from
 * a moderator's perspective ("All" here means review-or-flagged, the
 * working set of items needing a decision).
 *
 * Bearer-token leak avoidance: each row's action buttons live in the
 * `ModerationActions` client component, but the bearer never crosses
 * the server/client boundary. We define `'use server'` actions here
 * (acceptAction/rejectAction/dismissAction) that read the session
 * cookie server-side, call the API, and `revalidatePath` the queue.
 * The actions are passed in as props — only their reference is
 * serialised to the client island, not the token.
 *
 * After each action the cookie is re-read inside the action; if the
 * session has expired the action redirects to login.
 */

import Link from 'next/link';
import { AdminPageHeader } from '../_components/AdminPageHeader';
import type { Route } from 'next';
import { redirect } from 'next/navigation';
import { revalidatePath } from 'next/cache';
import {
  ApiCallError,
  acceptSubmission,
  dismissSubmissionFlag,
  getAdminSubmissionQueue,
  rejectSubmission,
  type SubmissionDto,
} from '@/lib/api';
import { logger } from '@/lib/logger';
import { getSession } from '@/lib/session';
import { StatusPill } from '../../submissions/_components/StatusPill';
import { ModerationActions } from './_components/ModerationActions';

const PAGE_LIMIT = 50;

type FilterId = 'review' | 'flagged' | 'all';

const FILTER_TABS: ReadonlyArray<{
  id: FilterId;
  label: string;
  /** Status param to send to the queue API. */
  apiStatus: 'review' | 'flagged' | 'all';
}> = [
  { id: 'review', label: 'In review', apiStatus: 'review' },
  { id: 'flagged', label: 'Flagged', apiStatus: 'flagged' },
  { id: 'all', label: 'All open', apiStatus: 'all' },
];

interface SearchParams {
  status?: string;
  offset?: string;
}

export default async function AdminSubmissionsPage(props: {
  searchParams: Promise<SearchParams>;
}) {
  const session = await getSession();
  if (!session) redirect('/auth/login?next=/admin/submissions');

  const params = await props.searchParams;
  const filter = parseFilter(params.status);
  const offset = parseOffset(params.offset);
  const activeStatus =
    FILTER_TABS.find((t) => t.id === filter)?.apiStatus ?? 'review';

  // Server actions — defined inside the component so they close over
  // nothing except the per-request scope. They read the session cookie
  // themselves; the bearer never leaves the server.
  async function acceptAction(formData: FormData) {
    'use server';
    const id = String(formData.get('id') ?? '');
    if (!id) throw new Error('missing submission id');
    const s = await getSession();
    if (!s) redirect('/auth/login?next=/admin/submissions');
    await acceptSubmission(s.token, id);
    revalidatePath('/admin/submissions');
    revalidatePath('/admin');
  }

  async function rejectAction(formData: FormData) {
    'use server';
    const id = String(formData.get('id') ?? '');
    const reason = String(formData.get('reason') ?? '').trim();
    if (!id) throw new Error('missing submission id');
    if (reason.length < 3) throw new Error('reason must be at least 3 chars');
    const s = await getSession();
    if (!s) redirect('/auth/login?next=/admin/submissions');
    await rejectSubmission(s.token, id, reason);
    revalidatePath('/admin/submissions');
    revalidatePath('/admin');
  }

  async function dismissAction(formData: FormData) {
    'use server';
    const id = String(formData.get('id') ?? '');
    if (!id) throw new Error('missing submission id');
    const s = await getSession();
    if (!s) redirect('/auth/login?next=/admin/submissions');
    await dismissSubmissionFlag(s.token, id);
    revalidatePath('/admin/submissions');
    revalidatePath('/admin');
  }

  // Probe each tab so we can render the "non-empty" dot indicator
  // without paying for full pages. Mirrors the user-facing pattern.
  let probeReview = false;
  let probeFlagged = false;
  let probeAll = false;
  // Settle each call independently (multi-endpoint dashboard invariant).
  // `main` is load-bearing — the queue listing IS the page — so its failure
  // stays fatal. The three probes only light filter-tab dots; a probe hiccup
  // must never 500 the console, it just greys that dot.
  const [mainResult, prResult, pfResult, paResult] = await Promise.allSettled([
    getAdminSubmissionQueue(session.token, {
      status: activeStatus,
      limit: PAGE_LIMIT,
      offset,
    }),
    getAdminSubmissionQueue(session.token, { status: 'review', limit: 1 }),
    getAdminSubmissionQueue(session.token, { status: 'flagged', limit: 1 }),
    getAdminSubmissionQueue(session.token, { status: 'all', limit: 1 }),
  ]);

  for (const [label, result] of [
    ['admin_submissions_probe_review', prResult],
    ['admin_submissions_probe_flagged', pfResult],
    ['admin_submissions_probe_all', paResult],
  ] as const) {
    if (result.status === 'rejected') {
      const status =
        result.reason instanceof ApiCallError
          ? result.reason.status
          : undefined;
      if (status === 401) redirect('/auth/login?next=/admin/submissions');
      if (status === 403) redirect('/me');
      logger.error(
        { err: result.reason, call: label, status },
        'admin submissions probe rejected',
      );
    }
  }
  probeReview =
    prResult.status === 'fulfilled' &&
    (prResult.value.items.length > 0 || prResult.value.has_more);
  probeFlagged =
    pfResult.status === 'fulfilled' &&
    (pfResult.value.items.length > 0 || pfResult.value.has_more);
  probeAll =
    paResult.status === 'fulfilled' &&
    (paResult.value.items.length > 0 || paResult.value.has_more);

  if (mainResult.status === 'rejected') {
    const status =
      mainResult.reason instanceof ApiCallError
        ? mainResult.reason.status
        : undefined;
    if (status === 401) redirect('/auth/login?next=/admin/submissions');
    if (status === 403) redirect('/me');
    throw mainResult.reason;
  }
  const listing = mainResult.value;

  const items = listing.items;
  const showOlder = listing.has_more;
  const showNewer = offset > 0;
  const heading = headingFor(filter);

  return (
    <div
      style={{ display: 'flex', flexDirection: 'column', gap: 20 }}
    >

      <AdminPageHeader
        eyebrow="Admin · moderation queue"
        title={heading}
        lede={
          <>
            Each decision is final-ish: accepted rules ship in the next
            parser update, rejected rules return to the submitter with a
            reason, and dismiss-flag clears reports without touching the
            rule.
          </>
        }
      />

      <nav
        aria-label="Moderation filters"
        style={{ display: 'flex', flexWrap: 'wrap', gap: 6 }}
      >
        {FILTER_TABS.map((t) => {
          const active = t.id === filter;
          const probed =
            t.id === 'review'
              ? probeReview
              : t.id === 'flagged'
                ? probeFlagged
                : probeAll;
          return (
            <Link
              key={t.id}
              href={buildHref({ filter: t.id })}
              data-active={active ? 'true' : undefined}
              style={{
                background: active ? 'var(--bg-elev)' : 'transparent',
                border: '1px solid',
                borderColor: active ? 'var(--border-strong)' : 'transparent',
                color: active ? 'var(--fg)' : 'var(--fg-muted)',
                padding: '8px 14px',
                borderRadius: 0,
                fontSize: 13,
                textDecoration: 'none',
                display: 'inline-flex',
                alignItems: 'center',
                gap: 8,
              }}
            >
              <span>{t.label}</span>
              <span
                aria-hidden="true"
                style={{
                  width: 6,
                  height: 6,
                  borderRadius: 0,
                  background: probed
                    ? 'var(--accent)'
                    : 'var(--border-strong)',
                }}
              />
            </Link>
          );
        })}
      </nav>

      {items.length === 0 ? (
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
            Nothing waiting in this bucket.
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
          {items.map((s) => (
            <li key={s.id}>
              <ModerationRow
                submission={s}
                acceptAction={acceptAction}
                rejectAction={rejectAction}
                dismissAction={dismissAction}
              />
            </li>
          ))}
        </ul>
      )}

      {(showOlder || showNewer) && (
        <nav
          aria-label="Queue pagination"
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
              href={buildHref({ filter, offset: offset + PAGE_LIMIT })}
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

function ModerationRow({
  submission,
  acceptAction,
  rejectAction,
  dismissAction,
}: {
  submission: SubmissionDto;
  acceptAction: (formData: FormData) => Promise<void>;
  rejectAction: (formData: FormData) => Promise<void>;
  dismissAction: (formData: FormData) => Promise<void>;
}) {
  return (
    <article
      className="ss-card"
      style={{
        display: 'flex',
        flexDirection: 'column',
        gap: 12,
        padding: '16px 18px',
      }}
    >
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 8,
          flexWrap: 'wrap',
        }}
      >
        <span
          className="mono"
          style={{ fontSize: 11, color: 'var(--fg-dim)' }}
        >
          {shortId(submission.id)}
        </span>
        <StatusPill status={submission.status} />
        {submission.flag_count > 0 && (
          <span className="ss-badge ss-badge--warn">
            {submission.flag_count}{' '}
            {submission.flag_count === 1 ? 'flag' : 'flags'}
          </span>
        )}
        <span
          className="mono"
          style={{ fontSize: 11, color: 'var(--accent)' }}
        >
          {submission.log_source}
        </span>
        <span
          style={{
            marginLeft: 'auto',
            fontSize: 11,
            color: 'var(--fg-dim)',
          }}
        >
          by{' '}
          <span className="mono" style={{ color: 'var(--fg-muted)' }}>
            {submission.submitter_handle}
          </span>{' '}
          · {formatRelativeTime(submission.created_at)}
        </span>
      </div>

      <div
        className="mono"
        style={{
          fontSize: 14,
          color: 'var(--accent)',
          wordBreak: 'break-word',
        }}
      >
        {submission.proposed_label}
      </div>

      <FieldRow label="Pattern" mono value={submission.pattern} />
      <FieldRow label="Sample" mono value={submission.sample_line} />
      <FieldRow label="Description" value={submission.description} />

      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'space-between',
          gap: 12,
          paddingTop: 6,
          borderTop: '1px solid var(--border)',
          flexWrap: 'wrap',
        }}
      >
        <Link
          href={
            `/submissions/${encodeURIComponent(submission.id)}` as Route
          }
          style={{
            fontSize: 12,
            color: 'var(--fg-muted)',
            textDecoration: 'none',
          }}
        >
          View public detail →
        </Link>
        <ModerationActions
          submissionId={submission.id}
          status={submission.status}
          acceptAction={acceptAction}
          rejectAction={rejectAction}
          dismissAction={dismissAction}
        />
      </div>
    </article>
  );
}

function FieldRow({
  label,
  value,
  mono = false,
}: {
  label: string;
  value: string;
  mono?: boolean;
}) {
  return (
    <div
      style={{
        display: 'grid',
        gridTemplateColumns: '90px 1fr',
        gap: 12,
        alignItems: 'baseline',
      }}
    >
      <div
        className="ss-eyebrow"
        style={{ fontSize: 10, color: 'var(--fg-dim)' }}
      >
        {label}
      </div>
      <div
        className={mono ? 'mono' : ''}
        style={{
          fontSize: 13,
          color: 'var(--fg)',
          wordBreak: 'break-word',
          whiteSpace: 'pre-wrap',
        }}
      >
        {value}
      </div>
    </div>
  );
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
  offset?: number;
}): Route {
  const qs = new URLSearchParams();
  // `review` is the default — omit so the bare URL is canonical.
  if (opts.filter !== 'review') qs.set('status', opts.filter);
  if (opts.offset !== undefined && opts.offset > 0) {
    qs.set('offset', String(opts.offset));
  }
  const suffix = qs.toString();
  return (suffix
    ? `/admin/submissions?${suffix}`
    : '/admin/submissions') as Route;
}

function headingFor(filter: FilterId): string {
  switch (filter) {
    case 'review':
      return 'In review';
    case 'flagged':
      return 'Flagged submissions';
    case 'all':
      return 'All open submissions';
  }
}

function shortId(id: string): string {
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
