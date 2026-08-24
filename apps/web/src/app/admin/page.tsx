import { Plane } from 'holo';
/**
 * Admin landing page — moderator-facing dashboard.
 *
 * Auth: the `/admin/layout.tsx` (D1 owns) gates the whole subtree on
 * moderator/admin role. We still call `getSession()` here for type
 * narrowing and a defensive redirect — the layout's redirect happens
 * first but a no-session render path would otherwise be a type error
 * when reading `session.token`.
 *
 * The two queue cards probe `getAdminSubmissionQueue` with `limit=1`
 * for each status bucket. Mirrors the probe pattern from
 * `app/submissions/page.tsx` (lines 71-100). We deliberately don't
 * surface counts — the API has no `total` yet — only "empty / not".
 */

import Link from 'next/link';
import type { Route } from 'next';
import { redirect } from 'next/navigation';
import {
  ApiCallError,
  getAdminSubmissionQueue,
} from '@/lib/api';
import { logger } from '@/lib/logger';
import { getSession } from '@/lib/session';
import { InstrumentStrip } from '@/components/hud/InstrumentStrip';

export default async function AdminLandingPage() {
  const session = await getSession();
  if (!session) redirect('/auth/login?next=/admin');

  // Two decorative queue-status probes. Per the multi-endpoint dashboard
  // invariant, settle each independently: one probe hiccup must not 500 the
  // whole moderation console — it just greys that card's dot. Auth failures
  // still redirect (the layout should have caught them first).
  let probeReview = false;
  let probeFlagged = false;
  const [reviewResult, flaggedResult] = await Promise.allSettled([
    getAdminSubmissionQueue(session.token, { status: 'review', limit: 1 }),
    getAdminSubmissionQueue(session.token, { status: 'flagged', limit: 1 }),
  ]);
  for (const [label, result] of [
    ['admin_queue_review', reviewResult],
    ['admin_queue_flagged', flaggedResult],
  ] as const) {
    if (result.status === 'rejected') {
      const status =
        result.reason instanceof ApiCallError
          ? result.reason.status
          : undefined;
      if (status === 401) redirect('/auth/login?next=/admin');
      if (status === 403) redirect('/me');
      logger.error(
        { err: result.reason, call: label, status },
        'admin landing probe rejected',
      );
    }
  }
  // `has_more` alone isn't enough — a bucket with exactly 1 item has
  // `has_more: false`. Treat "non-empty" as either flag; a failed probe
  // stays false (dot greys, reads "Nothing waiting").
  if (reviewResult.status === 'fulfilled') {
    probeReview =
      reviewResult.value.items.length > 0 || reviewResult.value.has_more;
  }
  if (flaggedResult.status === 'fulfilled') {
    probeFlagged =
      flaggedResult.value.items.length > 0 || flaggedResult.value.has_more;
  }

  return (
    <div
      style={{ display: 'flex', flexDirection: 'column', gap: 14 }}
    >

      <InstrumentStrip
        title={<h1 className="hud-tile__title" style={{ margin: 0, fontSize: 18 }}>Moderation</h1>}
        context="Admin · moderation console"
      />
      <p
        style={{
          margin: '6px 0 0',
          color: 'var(--fg-muted)',
          fontSize: 14,
          maxWidth: 640,
        }}
      >
        Triage community-submitted parser rules. Accept ships them in
        the next parser update; reject sends a reason back to the
        submitter; dismiss-flag clears community reports without
        changing rule status.
      </p>

      <div
        data-rspgrid="2"
        style={{
          display: 'grid',
          gridTemplateColumns: 'repeat(auto-fit, minmax(280px, 1fr))',
          gap: 16,
        }}
      >
        <QueueCard
          eyebrow="Triage queue"
          title="Submissions in review"
          description="Newly proposed parser rules awaiting a moderator decision."
          href={'/admin/submissions?status=review' as Route}
          nonEmpty={probeReview}
        />
        <QueueCard
          eyebrow="Community reports"
          title="Flagged submissions"
          description="Already-accepted patterns that the community has flagged for revisiting."
          href={'/admin/submissions?status=flagged' as Route}
          nonEmpty={probeFlagged}
        />
      </div>

      <Plane tilt="flat">
        <div className="ss-eyebrow" style={{ marginBottom: 6 }}>
          Recent admin actions
        </div>
        <h2
          style={{
            margin: 0,
            fontSize: 15,
            fontWeight: 600,
            letterSpacing: '-0.01em',
          }}
        >
          Open the audit log
        </h2>
        <p
          style={{
            margin: '10px 0 16px',
            color: 'var(--fg-muted)',
            fontSize: 13,
            lineHeight: 1.6,
          }}
        >
          Every state-changing API call writes one hash-chained row. The
          viewer is filterable by actor, action, and timestamp range.
        </p>
        <Link
          href={'/admin/audit' as Route}
          className="hp-btn hp-btn--ghost"
          style={{ textDecoration: 'none' }}
        >
          Audit log →
        </Link>
      </Plane>
    </div>
  );
}

function QueueCard({
  eyebrow,
  title,
  description,
  href,
  nonEmpty,
}: {
  eyebrow: string;
  title: string;
  description: string;
  href: Route;
  nonEmpty: boolean;
}) {
  return (
    <Link
      href={href}
      className="ss-card"
      style={{
        display: 'flex',
        flexDirection: 'column',
        gap: 10,
        padding: '13px 16px',
        textDecoration: 'none',
        color: 'inherit',
        minHeight: 160,
      }}
    >
      <div
        style={{
          display: 'flex',
          justifyContent: 'space-between',
          alignItems: 'center',
        }}
      >
        <div className="ss-eyebrow">{eyebrow}</div>
        <span
          aria-label={nonEmpty ? 'Has pending items' : 'Empty'}
          title={nonEmpty ? 'Has pending items' : 'Empty'}
          style={{
            width: 8,
            height: 8,
            borderRadius: 0,
            background: nonEmpty ? 'var(--accent)' : 'var(--border-strong)',
          }}
        />
      </div>
      <h2
        style={{
          margin: 0,
          fontSize: 15,
          fontWeight: 600,
          letterSpacing: '-0.01em',
        }}
      >
        {title}
      </h2>
      <p
        style={{
          margin: 0,
          color: 'var(--fg-muted)',
          fontSize: 13,
          lineHeight: 1.5,
        }}
      >
        {description}
      </p>
      <div
        style={{
          marginTop: 'auto',
          display: 'flex',
          alignItems: 'center',
          gap: 6,
          color: nonEmpty ? 'var(--accent)' : 'var(--fg-dim)',
          fontSize: 13,
        }}
      >
        <span>{nonEmpty ? 'Open queue' : 'Nothing waiting'}</span>
        <span aria-hidden="true">→</span>
      </div>
    </Link>
  );
}
