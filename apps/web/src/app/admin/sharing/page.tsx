import { Plane } from 'holo';
/**
 * Admin · Sharing overview.
 *
 * First of the five views called for in audit §05 ("Admin side · what
 * to build"). Wave W3 ships two:
 *   1. /admin/sharing            — this page (overview)
 *   2. /admin/sharing/audit      — sharing-scoped audit log
 *
 * Deferred to a later wave (TODO: see end of file):
 *   - /admin/sharing/reports     — needs a backend reports table that
 *                                  doesn't exist yet.
 *   - User detail sub-tab on /admin/users/[id]
 *   - Org detail sub-tab on /admin/orgs/[slug]
 *
 * Data source for v2 (post-W5):
 *   Real `/v1/admin/sharing/overview` + `/v1/admin/sharing/scope-histogram`
 *   endpoints. The overview returns:
 *     - active-share inventory off `share_metadata` (total / outbound /
 *       with-expiry / top-20 granters by active count),
 *     - last-30d audit-log totals for `share.granted`, `share.revoked`,
 *       `share.viewed` (already aggregated server-side, so we use the
 *       returned counts straight).
 *   The histogram returns the kind-distribution + per-tab usage for
 *   `kind = 'tabs'` rows.
 *
 *   This replaces the W3 proxy that tailed the audit log — that proxy
 *   could only ever see N recent rows, so the "active shares" card
 *   silently lied as soon as activity crossed N.
 *
 * Auth: parent layout (`/admin/layout.tsx`) gates the whole subtree
 * on moderator/admin role. We still call `getSession()` here for type
 * narrowing + a defensive redirect.
 */

import Link from 'next/link';
import type { Route } from 'next';
import { redirect } from 'next/navigation';
import {
  ApiCallError,
  getAdminSharingOverview,
  getAdminSharingScopeHistogram,
  type AdminSharingOverview,
  type ScopeHistogram,
} from '@/lib/api';
import { logger } from '@/lib/logger';
import { getSession } from '@/lib/session';
import { AdminPageHeader } from '../_components/AdminPageHeader';

export default async function AdminSharingOverviewPage() {
  const session = await getSession();
  if (!session) redirect('/auth/login?next=/admin/sharing');

  // Two parallel fetches: the overview (active inventory + 30d audit
  // totals) and the scope histogram. Both are read-only and gated on
  // moderator server-side; we issue them concurrently so the page
  // renders in one round-trip's wall-clock time.
  // Settle the two sections independently (multi-endpoint dashboard
  // invariant): an overview hiccup must not blank the scope histogram, and
  // vice versa. Each failed fetch degrades its own section to a
  // "temporarily unavailable" card; auth failures still redirect.
  const [overviewResult, histogramResult] = await Promise.allSettled([
    getAdminSharingOverview(session.token),
    getAdminSharingScopeHistogram(session.token),
  ]);
  for (const [label, result] of [
    ['admin_sharing_overview', overviewResult],
    ['admin_sharing_histogram', histogramResult],
  ] as const) {
    if (result.status === 'rejected') {
      const status =
        result.reason instanceof ApiCallError
          ? result.reason.status
          : undefined;
      if (status === 401) redirect('/auth/login?next=/admin/sharing');
      if (status === 403) redirect('/me');
      logger.error(
        { err: result.reason, call: label, status },
        'admin sharing section fetch rejected',
      );
    }
  }
  const overview: AdminSharingOverview | null =
    overviewResult.status === 'fulfilled' ? overviewResult.value : null;
  const histogram: ScopeHistogram | null =
    histogramResult.status === 'fulfilled' ? histogramResult.value : null;

  const netGrants30d = overview
    ? overview.total_grants_30d - overview.total_revocations_30d
    : 0;

  // Sum the four kind buckets so the histogram card can render a
  // total + percentage; immutable derivation rather than mutating
  // `histogram`.
  const histogramTotal = histogram
    ? histogram.full + histogram.timeline + histogram.aggregates + histogram.tabs
    : 0;

  return (
    <div
      style={{ display: 'flex', flexDirection: 'column', gap: 20 }}
    >

      <AdminPageHeader
        eyebrow="Admin · sharing"
        title="Sharing overview"
        lede={
          <>
            Live inventory of active shares plus the last 30 days of
            sharing-related audit activity. Counters refresh on every
            page load.
          </>
        }
      />

      {overview === null ? (
        <SectionUnavailable title="Active share inventory" />
      ) : (
      <>
      <section
        aria-label="Active share inventory"
        style={{
          display: 'grid',
          gridTemplateColumns: 'repeat(auto-fit, minmax(180px, 1fr))',
          gap: 12,
        }}
      >
        <StatCard
          eyebrow="Active shares"
          label="share_metadata"
          value={overview.active_shares_total}
          hint="No expiry, or expiry in the future"
        />
        <StatCard
          eyebrow="Outbound"
          label="user → user"
          value={overview.active_shares_outbound}
          hint="Org-direction grants don't have side-table rows yet"
        />
        <StatCard
          eyebrow="With expiry"
          label="explicit expires_at"
          value={overview.active_shares_with_expiry}
          hint="Excludes legacy no-expiry rows"
        />
        <StatCard
          eyebrow="Grants · 30d"
          label="share.granted"
          value={overview.total_grants_30d}
          hint={`Net: ${netGrants30d >= 0 ? '+' : ''}${netGrants30d}`}
          tone={netGrants30d >= 0 ? 'ok' : 'danger'}
        />
        <StatCard
          eyebrow="Revocations · 30d"
          label="share.revoked"
          value={overview.total_revocations_30d}
        />
        <StatCard
          eyebrow="Views · 30d"
          label="share.viewed"
          value={overview.total_views_30d}
          hint="Recipient reads in the last 30 days"
        />
      </section>

      <Plane tilt="flat">
        <div className="ss-eyebrow" style={{ marginBottom: 6 }}>
          Top granters
        </div>
        <h2
          style={{
            margin: 0,
            fontSize: 17,
            fontWeight: 600,
            letterSpacing: '-0.01em',
          }}
        >
          Most active sharers (live)
        </h2>
        <p
          style={{
            margin: '8px 0 16px',
            color: 'var(--fg-muted)',
            fontSize: 13,
            lineHeight: 1.5,
          }}
        >
          Owners ranked by the number of currently-active shares they
          hold in `share_metadata`. Capped at the top 20.
        </p>
        {overview.top_granters.length === 0 ? (
          <p
            style={{
              margin: 0,
              padding: '20px 0',
              color: 'var(--fg-muted)',
              fontSize: 13,
            }}
          >
            No active shares right now.
          </p>
        ) : (
          <table
            style={{
              width: '100%',
              borderCollapse: 'collapse',
              fontSize: 13,
            }}
          >
            <thead>
              <tr style={{ background: 'var(--bg-elev)' }}>
                <Th width="48px">#</Th>
                <Th>Handle</Th>
                <Th width="120px">Active</Th>
              </tr>
            </thead>
            <tbody>
              {overview.top_granters.map((row, i) => (
                <tr
                  key={row.handle}
                  style={{ borderBottom: '1px solid var(--border)' }}
                >
                  <Td>
                    <span
                      className="mono"
                      style={{ color: 'var(--fg-dim)' }}
                    >
                      {i + 1}
                    </span>
                  </Td>
                  <Td>
                    <Link
                      href={
                        (`/u/${encodeURIComponent(row.handle)}`) as Route
                      }
                      className="mono"
                      style={{ color: 'var(--accent)' }}
                    >
                      {row.handle}
                    </Link>
                  </Td>
                  <Td>
                    <span className="mono">{row.active_share_count}</span>
                  </Td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </Plane>
      </>
      )}

      {histogram === null ? (
        <SectionUnavailable title="Scope distribution" />
      ) : (
      <Plane tilt="flat">
        <div className="ss-eyebrow" style={{ marginBottom: 6 }}>
          Scope distribution
        </div>
        <h2
          style={{
            margin: 0,
            fontSize: 17,
            fontWeight: 600,
            letterSpacing: '-0.01em',
          }}
        >
          Active-share scope kinds
        </h2>
        <p
          style={{
            margin: '8px 0 16px',
            color: 'var(--fg-muted)',
            fontSize: 13,
            lineHeight: 1.5,
          }}
        >
          Distribution of <code>scope-&gt;&gt;&apos;kind&apos;</code>{' '}
          across the {histogramTotal.toLocaleString()} currently-active
          shares. Legacy rows (NULL scope) count under <code>full</code>.
        </p>
        {histogramTotal === 0 ? (
          <p
            style={{
              margin: 0,
              padding: '20px 0',
              color: 'var(--fg-muted)',
              fontSize: 13,
            }}
          >
            No active shares to chart.
          </p>
        ) : (
          <div
            style={{
              display: 'grid',
              gridTemplateColumns:
                'repeat(auto-fit, minmax(140px, 1fr))',
              gap: 12,
            }}
          >
            <ScopeBucket
              label="full"
              value={histogram.full}
              total={histogramTotal}
            />
            <ScopeBucket
              label="timeline"
              value={histogram.timeline}
              total={histogramTotal}
            />
            <ScopeBucket
              label="aggregates"
              value={histogram.aggregates}
              total={histogramTotal}
            />
            <ScopeBucket
              label="tabs"
              value={histogram.tabs}
              total={histogramTotal}
            />
          </div>
        )}
        {Object.keys(histogram.tab_usage).length > 0 && (
          <div style={{ marginTop: 20 }}>
            <div className="ss-eyebrow" style={{ marginBottom: 8 }}>
              Tab usage (kind = tabs)
            </div>
            <table
              style={{
                width: '100%',
                borderCollapse: 'collapse',
                fontSize: 13,
              }}
            >
              <thead>
                <tr style={{ background: 'var(--bg-elev)' }}>
                  <Th>Tab</Th>
                  <Th width="120px">Shares</Th>
                </tr>
              </thead>
              <tbody>
                {Object.entries(histogram.tab_usage)
                  .sort((a, b) => b[1] - a[1])
                  .map(([tab, count]) => (
                    <tr
                      key={tab}
                      style={{
                        borderBottom: '1px solid var(--border)',
                      }}
                    >
                      <Td>
                        <span className="mono">{tab}</span>
                      </Td>
                      <Td>
                        <span className="mono">{count}</span>
                      </Td>
                    </tr>
                  ))}
              </tbody>
            </table>
          </div>
        )}
      </Plane>
      )}

      <nav
        aria-label="Sharing sub-views"
        style={{
          display: 'flex',
          gap: 8,
          flexWrap: 'wrap',
        }}
      >
        <Link
          href={'/admin/sharing/audit' as Route}
          className="hp-btn hp-btn--ghost"
          style={{ textDecoration: 'none' }}
        >
          Sharing audit log →
        </Link>
        <Link
          href={'/admin/sharing/reports' as Route}
          className="hp-btn hp-btn--ghost"
          style={{ textDecoration: 'none' }}
        >
          Reports queue →
        </Link>
      </nav>
    </div>
  );
}

/** Section-level fallback when one of the two parallel fetches failed.
 *  The other section still renders — that's the whole point of settling
 *  each call independently rather than failing the page. */
function SectionUnavailable({ title }: { title: string }) {
  return (
    <Plane tilt="flat">
      <div className="ss-eyebrow" style={{ marginBottom: 6 }}>
        {title}
      </div>
      <p
        style={{
          margin: '8px 0 0',
          color: 'var(--fg-muted)',
          fontSize: 13,
          lineHeight: 1.5,
        }}
      >
        Temporarily unavailable — this section failed to load. The rest of
        the page is unaffected; refresh to retry.
      </p>
    </Plane>
  );
}

function StatCard({
  eyebrow,
  label,
  value,
  hint,
  tone,
}: {
  eyebrow: string;
  label: string;
  value: number;
  hint?: string;
  tone?: 'ok' | 'danger';
}) {
  const accent =
    tone === 'danger'
      ? 'var(--danger)'
      : tone === 'ok'
        ? 'var(--accent)'
        : 'var(--fg)';
  return (
    <Plane tilt="flat">
      <div className="ss-eyebrow">{eyebrow}</div>
      <div
        style={{
          fontSize: 26,
          fontWeight: 600,
          letterSpacing: '-0.02em',
          color: accent,
        }}
      >
        {value.toLocaleString()}
      </div>
      <div
        className="mono hp-field__hint"
      >
        {label}
      </div>
      {hint && (
        <div
          style={{
            color: 'var(--fg-muted)',
            fontSize: 11,
            lineHeight: 1.4,
            marginTop: 2,
          }}
        >
          {hint}
        </div>
      )}
    </Plane>
  );
}

/**
 * One bucket card in the scope-kind histogram. Renders the raw count
 * plus a percentage of the (non-zero) total so the four buckets are
 * directly comparable at a glance.
 */
function ScopeBucket({
  label,
  value,
  total,
}: {
  label: string;
  value: number;
  total: number;
}) {
  const pct = total > 0 ? Math.round((value / total) * 100) : 0;
  return (
    <div
      style={{
        border: '1px solid var(--border)',
        borderRadius: 0,
        padding: '14px 16px',
        display: 'flex',
        flexDirection: 'column',
        gap: 4,
      }}
    >
      <div
        className="mono hp-field__hint"
      >
        {label}
      </div>
      <div
        style={{
          fontSize: 22,
          fontWeight: 600,
          letterSpacing: '-0.02em',
        }}
      >
        {value.toLocaleString()}
      </div>
      <div className="hp-field__hint">
        {pct}% of active
      </div>
    </div>
  );
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

// TODO(W3 follow-up wave): three more views from audit §05.
//
//   1. /admin/sharing/reports
//      Triage queue for `share.reported` events. Needs a backend
//      `share_reports` table (status: open/acknowledged/resolved,
//      reporter_id, share_id, reason, resolved_by, resolved_at).
//      No backend table exists today — W3-S scope is per-share
//      scopes + share.viewed, not the reports queue.
//
//   2. User detail sub-tab on /admin/users/[id]/page.tsx
//      Add a "Sharing" tab alongside the existing role-grant UI. It
//      should list (a) the user's outbound grants, (b) their inbound
//      grants, (c) sharing-related audit rows scoped to actor_handle
//      = this user. Needs a `/v1/admin/users/{id}/shares` endpoint
//      (or two existing ones called with admin impersonation).
//
//   3. Org detail sub-tab on /admin/orgs/[slug]/page.tsx
//      List members who've granted the org access (org_shares)
//      plus a per-org slice of share.created/revoked events.
//
//   Per W3 scope these are noted in the audit report but not built.
//   See the report for cross-references to the exact files where
//   the sub-tabs should slot in.

// Force dynamic render: the underlying counters change on every
// state-changing API call, so a static cache would lie within
// seconds.
export const dynamic = 'force-dynamic';
