/**
 * Submission detail — pattern + samples on the left, vote/flag/withdraw
 * controls + lifecycle on the right. Mirrors the auth + error pattern
 * of `app/dashboard/page.tsx`.
 *
 * Mutations (vote, flag, withdraw) are inline server actions on this
 * page — they call the API helper, revalidate this route + the list
 * route, and bounce errors back through search params for redirect-
 * based feedback (matching the convention used in `app/orgs/[slug]`).
 *
 * <!-- TODO: comments come in a follow-up wave — backend has no
 *      comment endpoints yet, so the design's Discussion card is
 *      omitted entirely. -->
 */

import Link from 'next/link';
import type { Route } from 'next';
import { revalidatePath } from 'next/cache';
import { redirect } from 'next/navigation';
import {
  ApiCallError,
  flagSubmission,
  getSubmission,
  voteOnSubmission,
  withdrawSubmission,
  type SubmissionDto,
} from '@/lib/api';
import { logger } from '@/lib/logger';
import { getSession } from '@/lib/session';
import { StatusPill } from '../_components/StatusPill';
import { WithdrawForm } from './_components/WithdrawForm';

export const metadata = { title: "Submission" };

interface SearchParams {
  status?: string;
  error?: string;
}

export default async function SubmissionDetailPage(props: {
  params: Promise<{ id: string }>;
  searchParams: Promise<SearchParams>;
}) {
  const session = await getSession();
  const { id } = await props.params;
  const search = await props.searchParams;
  const nextPath = `/submissions/${encodeURIComponent(id)}` as Route;
  const loginHref =
    `/auth/login?next=${nextPath}` as Route;

  if (!session) redirect(loginHref);

  let submission: SubmissionDto;
  try {
    submission = await getSubmission(session.token, id);
  } catch (e) {
    if (e instanceof ApiCallError) {
      if (e.status === 401) redirect(loginHref);
      if (e.status === 404) {
        return <NotFoundView />;
      }
    }
    throw e;
  }

  const isMine =
    submission.submitter_id === session.userId ||
    submission.submitter_handle === session.claimedHandle;
  const canWithdraw = isMine && submission.status === 'review';
  const escalated = search.status === 'flag_escalated';

  // -- Server actions (inline, co-located so they can close over `id`).

  async function voteAction(formData: FormData) {
    'use server';
    const s = await getSession();
    if (!s) redirect(loginHref);
    const want = formData.get('vote') === 'true';
    try {
      await voteOnSubmission(s.token, id, want);
    } catch (err) {
      if (err instanceof ApiCallError) {
        if (err.status === 401) redirect(loginHref);
        logger.error({ err, id }, 'vote on submission failed');
        redirect(
          `${nextPath}?error=${encodeURIComponent(err.body.error)}` as Route,
        );
      }
      throw err;
    }
    revalidatePath(nextPath);
    revalidatePath('/submissions');
    redirect(nextPath);
  }

  async function flagAction(formData: FormData) {
    'use server';
    const s = await getSession();
    if (!s) redirect(loginHref);
    const reasonRaw = formData.get('reason');
    const reason =
      typeof reasonRaw === 'string' && reasonRaw.trim() !== ''
        ? reasonRaw.trim()
        : undefined;
    let didEscalate = false;
    try {
      const resp = await flagSubmission(s.token, id, reason);
      didEscalate = resp.escalated;
    } catch (err) {
      if (err instanceof ApiCallError) {
        if (err.status === 401) redirect(loginHref);
        logger.error({ err, id }, 'flag submission failed');
        redirect(
          `${nextPath}?error=${encodeURIComponent(err.body.error)}` as Route,
        );
      }
      throw err;
    }
    revalidatePath(nextPath);
    revalidatePath('/submissions');
    redirect(
      (didEscalate
        ? `${nextPath}?status=flag_escalated`
        : nextPath) as Route,
    );
  }

  async function withdrawAction() {
    'use server';
    const s = await getSession();
    if (!s) redirect(loginHref);
    try {
      await withdrawSubmission(s.token, id);
    } catch (err) {
      if (err instanceof ApiCallError) {
        if (err.status === 401) redirect(loginHref);
        logger.error({ err, id }, 'withdraw submission failed');
        redirect(
          `${nextPath}?error=${encodeURIComponent(err.body.error)}` as Route,
        );
      }
      throw err;
    }
    revalidatePath(nextPath);
    revalidatePath('/submissions');
    redirect('/submissions?status=withdrawn' as Route);
  }

  return (
    <div
      className="ss-screen-enter"
      style={{ display: 'flex', flexDirection: 'column', gap: 20 }}
    >
      <div>
        <Link
          href={'/submissions' as Route}
          className="ss-btn ss-btn--ghost"
          style={{ display: 'inline-flex', alignItems: 'center', gap: 6 }}
        >
          ← All submissions
        </Link>
      </div>

      {escalated && (
        <div
          role="status"
          className="ss-card"
          style={{
            padding: '12px 16px',
            background: 'color-mix(in oklab, var(--warn) 10%, transparent)',
            borderColor: 'color-mix(in oklab, var(--warn) 40%, transparent)',
            color: 'var(--warn)',
            fontSize: 13,
          }}
        >
          This submission has been escalated to moderator review.
        </div>
      )}

      {search.error && (
        <div
          role="alert"
          className="ss-card"
          style={{
            padding: '12px 16px',
            background: 'color-mix(in oklab, var(--danger) 10%, transparent)',
            borderColor:
              'color-mix(in oklab, var(--danger) 40%, transparent)',
            color: 'var(--danger)',
            fontSize: 13,
          }}
        >
          Action failed: {search.error}
        </div>
      )}

      <header>
        <div
          style={{
            display: 'flex',
            alignItems: 'center',
            gap: 8,
            marginBottom: 10,
            flexWrap: 'wrap',
          }}
        >
          <span
            className="mono"
            style={{ fontSize: 12, color: 'var(--fg-dim)' }}
          >
            {shortId(submission.id)}
          </span>
          {isMine && (
            <span className="ss-badge ss-badge--accent">Mine</span>
          )}
          <StatusPill status={submission.status} />
          {submission.flag_count > 0 && (
            <span className="ss-badge ss-badge--warn">
              {submission.flag_count}{' '}
              {submission.flag_count === 1 ? 'flag' : 'flags'}
            </span>
          )}
        </div>
        <h1
          className="mono"
          style={{
            margin: 0,
            fontSize: 28,
            fontWeight: 500,
            letterSpacing: '-0.01em',
            color: 'var(--accent)',
            wordBreak: 'break-all',
          }}
        >
          {submission.proposed_label}
        </h1>
        <p
          style={{
            margin: '10px 0 0',
            color: 'var(--fg-muted)',
            fontSize: 14,
            maxWidth: 720,
            fontStyle: 'italic',
          }}
        >
          {submission.description}
        </p>
      </header>

      <div
        data-rspgrid="2"
        style={{
          display: 'grid',
          gridTemplateColumns: '2fr 1fr',
          gap: 20,
          alignItems: 'start',
        }}
      >
        {/* Left column ------------------------------------------- */}
        <div
          style={{
            display: 'flex',
            flexDirection: 'column',
            gap: 20,
            minWidth: 0,
          }}
        >
          <section className="ss-card" style={{ padding: '20px 24px' }}>
            <div className="ss-eyebrow" style={{ marginBottom: 6 }}>
              Raw pattern
            </div>
            <h2
              style={{
                margin: '0 0 12px',
                fontSize: 17,
                fontWeight: 600,
                letterSpacing: '-0.01em',
              }}
            >
              What the client matched
            </h2>
            <pre
              className="mono"
              style={{
                fontSize: 13,
                color: 'var(--fg)',
                background: 'var(--bg)',
                border: '1px solid var(--border)',
                borderRadius: 0,
                padding: '14px 16px',
                whiteSpace: 'pre-wrap',
                wordBreak: 'break-all',
                margin: 0,
              }}
            >
              {submission.pattern}
            </pre>
            <div
              style={{
                marginTop: 10,
                fontSize: 12,
                color: 'var(--fg-dim)',
              }}
            >
              Source:{' '}
              <span className="mono">{submission.log_source}</span>
            </div>
          </section>

          <section className="ss-card" style={{ padding: '20px 24px' }}>
            <div className="ss-eyebrow" style={{ marginBottom: 6 }}>
              Sample match
            </div>
            <h2
              style={{
                margin: '0 0 12px',
                fontSize: 17,
                fontWeight: 600,
                letterSpacing: '-0.01em',
              }}
            >
              A real line this caught
            </h2>
            <pre
              className="mono"
              style={{
                fontSize: 12,
                color: 'var(--fg-muted)',
                background: 'var(--bg)',
                border: '1px solid var(--border)',
                borderRadius: 0,
                padding: '14px 16px',
                whiteSpace: 'pre-wrap',
                wordBreak: 'break-all',
                margin: 0,
              }}
            >
              {submission.sample_line}
            </pre>
          </section>

          {submission.rejection_reason && (
            <section
              className="ss-card"
              style={{ padding: '20px 24px' }}
            >
              <div className="ss-eyebrow" style={{ marginBottom: 6 }}>
                Rejection reason
              </div>
              <p
                style={{
                  margin: 0,
                  fontSize: 13,
                  color: 'var(--fg-muted)',
                  lineHeight: 1.55,
                }}
              >
                {submission.rejection_reason}
              </p>
            </section>
          )}
        </div>

        {/* Right column ------------------------------------------ */}
        <div
          style={{
            display: 'flex',
            flexDirection: 'column',
            gap: 20,
            minWidth: 0,
          }}
        >
          <section className="ss-card" style={{ padding: '20px 24px' }}>
            <div className="ss-eyebrow" style={{ marginBottom: 6 }}>
              Your input
            </div>
            <h2
              style={{
                margin: '0 0 12px',
                fontSize: 17,
                fontWeight: 600,
                letterSpacing: '-0.01em',
              }}
            >
              Help triage
            </h2>

            <div
              style={{
                display: 'flex',
                flexDirection: 'column',
                gap: 10,
              }}
            >
              {/* Vote */}
              <form action={voteAction}>
                <input
                  type="hidden"
                  name="vote"
                  value={submission.viewer_voted ? 'false' : 'true'}
                />
                <button
                  type="submit"
                  className="ss-btn"
                  style={{
                    background: submission.viewer_voted
                      ? 'var(--accent)'
                      : 'var(--bg-elev)',
                    color: submission.viewer_voted
                      ? 'var(--accent-fg)'
                      : 'var(--fg)',
                    border: '1px solid',
                    borderColor: submission.viewer_voted
                      ? 'var(--accent)'
                      : 'var(--border)',
                    borderRadius: 0,
                    padding: '12px 14px',
                    width: '100%',
                    fontSize: 13,
                    fontWeight: 500,
                    display: 'flex',
                    alignItems: 'center',
                    justifyContent: 'space-between',
                    gap: 10,
                  }}
                >
                  <span>
                    {submission.viewer_voted
                      ? '✓ Voted to prioritize'
                      : 'Vote to prioritize'}
                  </span>
                  <span className="mono" style={{ fontSize: 12 }}>
                    {submission.vote_count.toLocaleString()}
                  </span>
                </button>
              </form>

              {/* Flag */}
              <FlagDisclosure
                flagAction={flagAction}
                viewerFlagged={submission.viewer_flagged}
                flagCount={submission.flag_count}
              />

              {canWithdraw && (
                <WithdrawForm withdrawAction={withdrawAction} />
              )}

              <p
                style={{
                  fontSize: 11,
                  color: 'var(--fg-dim)',
                  padding: '10px 12px',
                  background: 'var(--bg)',
                  border: '1px solid var(--border)',
                  borderRadius: 0,
                  lineHeight: 1.5,
                  margin: 0,
                }}
              >
                Flag if the pattern catches the wrong thing or the
                proposed name is misleading. Repeated flags from
                verified handles auto-escalate to a moderator.
              </p>
            </div>
          </section>

          <section className="ss-card" style={{ padding: '20px 24px' }}>
            <div className="ss-eyebrow" style={{ marginBottom: 6 }}>
              Submission
            </div>
            <h2
              style={{
                margin: '0 0 12px',
                fontSize: 17,
                fontWeight: 600,
                letterSpacing: '-0.01em',
              }}
            >
              Details
            </h2>
            <KV
              rows={[
                {
                  k: 'Submitted by',
                  v: (
                    <span className="mono">
                      {submission.submitter_handle}
                    </span>
                  ),
                },
                {
                  k: 'Submitted',
                  v: formatRelativeTime(submission.created_at),
                },
                {
                  k: 'Total votes',
                  v: (
                    <span className="mono">
                      {submission.vote_count.toLocaleString()}
                    </span>
                  ),
                },
                {
                  k: 'Total flags',
                  v: (
                    <span className="mono">
                      {submission.flag_count.toLocaleString()}
                    </span>
                  ),
                },
                {
                  k: 'Status',
                  v: <StatusPill status={submission.status} />,
                },
              ]}
            />
          </section>

          <section className="ss-card" style={{ padding: '20px 24px' }}>
            <div className="ss-eyebrow" style={{ marginBottom: 6 }}>
              What happens next
            </div>
            <h2
              style={{
                margin: '0 0 12px',
                fontSize: 17,
                fontWeight: 600,
                letterSpacing: '-0.01em',
              }}
            >
              Lifecycle
            </h2>
            <Lifecycle status={submission.status} />
          </section>
        </div>
      </div>
    </div>
  );
}

// -- Flag disclosure ------------------------------------------------

function FlagDisclosure({
  flagAction,
  viewerFlagged,
  flagCount,
}: {
  flagAction: (formData: FormData) => Promise<void>;
  viewerFlagged: boolean;
  flagCount: number;
}) {
  // Use a native <details> for the disclosure so this stays a server
  // component — no client JS needed for the open/close toggle.
  return (
    <details>
      <summary
        className="ss-btn ss-btn--ghost"
        style={{
          listStyle: 'none',
          cursor: 'pointer',
          width: '100%',
          justifyContent: 'flex-start',
          borderColor: viewerFlagged ? 'var(--danger)' : undefined,
          color: viewerFlagged ? 'var(--danger)' : undefined,
        }}
      >
        {viewerFlagged
          ? `✓ Flagged · ${flagCount}`
          : `Flag as incorrect${flagCount > 0 ? ` · ${flagCount}` : ''}`}
      </summary>
      <form
        action={flagAction}
        style={{
          marginTop: 10,
          display: 'flex',
          flexDirection: 'column',
          gap: 8,
        }}
      >
        <label
          htmlFor="flag-reason"
          style={{
            fontSize: 12,
            color: 'var(--fg-muted)',
          }}
        >
          Optional reason
        </label>
        <textarea
          id="flag-reason"
          name="reason"
          rows={3}
          placeholder="Why is this submission incorrect?"
          style={{
            background: 'var(--bg)',
            border: '1px solid var(--border)',
            borderRadius: 0,
            color: 'var(--fg)',
            font: 'inherit',
            fontSize: 13,
            padding: '8px 10px',
            resize: 'vertical',
          }}
        />
        <button type="submit" className="ss-btn ss-btn--ghost">
          Submit flag
        </button>
      </form>
    </details>
  );
}

// -- Key/value list -------------------------------------------------

function KV({
  rows,
}: {
  rows: Array<{ k: string; v: React.ReactNode }>;
}) {
  return (
    <dl
      style={{
        display: 'grid',
        gridTemplateColumns: 'auto 1fr',
        gap: '8px 16px',
        margin: 0,
        fontSize: 13,
      }}
    >
      {rows.map((r) => (
        <Row key={r.k} k={r.k} v={r.v} />
      ))}
    </dl>
  );
}

function Row({ k, v }: { k: string; v: React.ReactNode }) {
  return (
    <>
      <dt style={{ color: 'var(--fg-dim)' }}>{k}</dt>
      <dd style={{ margin: 0, color: 'var(--fg)' }}>{v}</dd>
    </>
  );
}

// -- Lifecycle ------------------------------------------------------

function Lifecycle({ status }: { status: string }) {
  // Phase 1 — submitted — is always done by definition.
  // Phase 2 — community vote — active while still in `review`.
  // Phase 3 — mod review — active when escalated (`flagged`) or once
  //           `accepted` (mods have signed off).
  // Phase 4 — ships — only `shipped`.
  const phases: Array<{
    label: string;
    note: string;
    state: 'done' | 'active' | 'pending';
  }> = [
    { label: 'Submitted', note: 'Posted to the queue', state: 'done' },
    {
      label: 'Community vote',
      note: 'Vote to advance',
      state:
        status === 'review'
          ? 'active'
          : status === 'withdrawn' || status === 'rejected'
            ? 'pending'
            : 'done',
    },
    {
      label: 'Mod review',
      note: 'Flagged or accepted submissions go through here',
      state:
        status === 'flagged'
          ? 'active'
          : status === 'accepted' || status === 'shipped'
            ? 'done'
            : 'pending',
    },
    {
      label: 'Ships in update',
      note: 'Lands in the next parser release',
      state:
        status === 'shipped'
          ? 'done'
          : status === 'accepted'
            ? 'active'
            : 'pending',
    },
  ];

  return (
    <ol
      style={{
        listStyle: 'none',
        margin: 0,
        padding: 0,
        display: 'flex',
        flexDirection: 'column',
        gap: 12,
      }}
    >
      {phases.map((p, i) => (
        <li
          key={i}
          style={{
            display: 'flex',
            gap: 12,
            alignItems: 'center',
          }}
        >
          <span
            style={{
              width: 10,
              height: 10,
              borderRadius: 0,
              background:
                p.state === 'done'
                  ? 'var(--ok)'
                  : p.state === 'active'
                    ? 'var(--accent)'
                    : 'var(--bg-elev)',
              border: '1px solid',
              borderColor:
                p.state === 'active'
                  ? 'var(--accent)'
                  : p.state === 'done'
                    ? 'var(--ok)'
                    : 'var(--border)',
              boxShadow:
                p.state === 'active'
                  ? '0 0 0 4px color-mix(in oklab, var(--accent) 20%, transparent)'
                  : 'none',
              flexShrink: 0,
            }}
            aria-hidden="true"
          />
          <div style={{ flex: 1, minWidth: 0 }}>
            <div
              style={{
                fontSize: 13,
                color:
                  p.state === 'pending' ? 'var(--fg-muted)' : 'var(--fg)',
              }}
            >
              {p.label}
            </div>
            <div style={{ fontSize: 11, color: 'var(--fg-dim)' }}>
              {p.note}
            </div>
          </div>
        </li>
      ))}
    </ol>
  );
}

// -- 404 view -------------------------------------------------------

function NotFoundView() {
  return (
    <div
      className="ss-screen-enter"
      style={{ display: 'flex', flexDirection: 'column', gap: 20 }}
    >
      <header>
        <div className="ss-eyebrow" style={{ marginBottom: 8 }}>
          Submission
        </div>
        <h1
          style={{
            margin: 0,
            fontSize: 32,
            fontWeight: 600,
            letterSpacing: '-0.02em',
          }}
        >
          Not found
        </h1>
        <p
          style={{
            margin: '6px 0 0',
            color: 'var(--fg-muted)',
            fontSize: 14,
          }}
        >
          Either this submission doesn&apos;t exist or it&apos;s been
          withdrawn.
        </p>
      </header>
      <section className="ss-card" style={{ padding: '20px 24px' }}>
        <Link
          href={'/submissions' as Route}
          className="ss-btn ss-btn--ghost"
        >
          ← Back to submissions
        </Link>
      </section>
    </div>
  );
}

// -- Helpers --------------------------------------------------------

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
