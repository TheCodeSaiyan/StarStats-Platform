/**
 * Admin · User detail.
 *
 * Surfaces the AdminUserDto + provides role grant/revoke forms.
 * Mutations use Server Actions that hit POST/DELETE on
 * /v1/admin/users/:id/roles. Status feedback flows back through the
 * URL so the page works without client JS.
 */

import Link from 'next/link';
import { AdminPageHeader } from '../../_components/AdminPageHeader';
import {
  AdminTable,
  type AdminTableColumn,
} from '../../_components/AdminTable';
import { DeleteAccountPanel } from './_components/DeleteAccountPanel';
import { RestrictionPanel } from './_components/RestrictionPanel';
import {
  SyncChip,
  relativeTime,
  retentionWindow,
} from '../../_components/user-activity';
import type { Route } from 'next';
import { redirect } from 'next/navigation';
import {
  ApiCallError,
  getAdminUser,
  getAdminUserSharingContext,
  grantAdminUserRole,
  revokeAdminUserRole,
  setAdminUserRestrictions,
  clearAdminUserRestrictions,
  deleteAdminUser,
  type AdminUserDetailDto,
  type AdminUserDeviceDto,
  type AdminUserEventTypeCountDto,
  type UserShareEdge,
  type UserSharingContext,
} from '@/lib/api';
import { logger } from '@/lib/logger';
import { getSession } from '@/lib/session';

interface PageProps {
  params: Promise<{ id: string }>;
  searchParams: Promise<{ status?: string; error?: string }>;
}

const STATUS_MESSAGES: Record<string, string> = {
  role_granted: 'Role granted.',
  role_revoked: 'Role revoked.',
  no_change: 'No change — the user already had that role state.',
  restricted: 'Restrictions applied.',
  reinstated: 'Restrictions lifted. Revoked shares were not restored.',
};

const ERROR_MESSAGES: Record<string, string> = {
  invalid_role: 'Invalid role. Pick moderator or admin.',
  reason_too_long: 'Reason too long (max 280 characters).',
  user_not_found: 'User no longer exists.',
  cannot_revoke_own_admin: "You can't revoke your own admin role.",
  forbidden: 'Admin role required for that change.',
  reason_required: 'A reason is required — it is shown to the user.',
  no_capabilities_selected:
    'Pick at least one capability to block, or use Reinstate to lift.',
  cannot_restrict_yourself: "You can't restrict your own account.",
  cannot_restrict_an_admin: "You can't restrict an admin.",
  cannot_delete_yourself: "You can't delete your own account.",
  cannot_delete_an_admin: "You can't delete an admin.",
  confirm_mismatch: 'The handle you typed did not match. Nothing was deleted.',
  unexpected: 'Something went wrong. Try again.',
};

export default async function AdminUserDetailPage(props: PageProps) {
  const session = await getSession();
  if (!session) redirect('/auth/login?next=/admin/users');

  const { id } = await props.params;
  const params = await props.searchParams;
  const status = params.status;
  const errorCode = params.error;

  let detail: AdminUserDetailDto;
  let sharing: UserSharingContext | null = null;
  try {
    detail = await getAdminUser(session.token, id);
  } catch (e) {
    if (e instanceof ApiCallError && e.status === 401) {
      redirect('/auth/login?next=/admin/users');
    }
    if (e instanceof ApiCallError && e.status === 403) {
      redirect('/me');
    }
    if (e instanceof ApiCallError && e.status === 404) {
      redirect('/admin/users?error=user_not_found');
    }
    throw e;
  }

  // The detail DTO flattens AdminUserDto at the top level and adds the
  // three insight blocks alongside it.
  const user = detail;
  const devices = detail.devices;
  const eventTypeCounts = detail.event_type_counts;
  const retention = detail.retention;

  // Audit v2.1 §C — per-user sharing context. Fail-soft so a hiccup
  // in the sharing endpoint doesn't block the role-management page;
  // the sub-tab just renders an empty state instead.
  try {
    sharing = await getAdminUserSharingContext(
      session.token,
      user.claimed_handle,
    );
  } catch (e) {
    logger.warn(
      { err: e, handle: user.claimed_handle },
      'admin user sharing context fetch failed',
    );
  }

  const isAdmin = session.staffRoles.some((r) => r === 'admin');
  const isModerator = user.staff_roles.includes('moderator');
  const isAdminTarget = user.staff_roles.includes('admin');
  const isSelf =
    session.claimedHandle.toLowerCase() ===
    user.claimed_handle.toLowerCase();

  async function grantAction(formData: FormData) {
    'use server';
    const s = await getSession();
    if (!s) redirect('/auth/login?next=/admin/users');
    const role = String(formData.get('role') ?? '').trim();
    const reason =
      String(formData.get('reason') ?? '').trim() || undefined;
    let changed: boolean;
    try {
      const res = await grantAdminUserRole(s.token, id, { role, reason });
      changed = res.changed;
    } catch (e) {
      if (e instanceof ApiCallError) {
        if (e.status === 401) redirect('/auth/login?next=/admin/users');
        if (e.status === 403) redirect(`/admin/users/${id}?error=forbidden`);
        if (e.status === 404) redirect(`/admin/users?error=user_not_found`);
        if (e.status === 400)
          redirect(
            `/admin/users/${id}?error=${encodeURIComponent(e.body.error)}`,
          );
      }
      logger.error({ err: e }, 'admin grant role failed');
      redirect(`/admin/users/${id}?error=unexpected`);
    }
    // Outside the try: Next implements `redirect()` by throwing a
    // NEXT_REDIRECT sentinel, so calling it above would have been
    // caught below and rerouted every SUCCESSFUL grant to
    // `?error=unexpected`. Same fix as /orgs/new and the auth actions.
    redirect(
      `/admin/users/${id}?status=${changed ? 'role_granted' : 'no_change'}`,
    );
  }

  async function revokeAction(formData: FormData) {
    'use server';
    const s = await getSession();
    if (!s) redirect('/auth/login?next=/admin/users');
    const role = String(formData.get('role') ?? '') as 'moderator' | 'admin';
    if (role !== 'moderator' && role !== 'admin') {
      redirect(`/admin/users/${id}?error=invalid_role`);
    }
    let changed: boolean;
    try {
      const res = await revokeAdminUserRole(s.token, id, role);
      changed = res.changed;
    } catch (e) {
      if (e instanceof ApiCallError) {
        if (e.status === 401) redirect('/auth/login?next=/admin/users');
        if (e.status === 403) redirect(`/admin/users/${id}?error=forbidden`);
        if (e.status === 404) redirect(`/admin/users?error=user_not_found`);
        if (e.status === 400)
          redirect(
            `/admin/users/${id}?error=${encodeURIComponent(e.body.error)}`,
          );
      }
      logger.error({ err: e }, 'admin revoke role failed');
      redirect(`/admin/users/${id}?error=unexpected`);
    }
    // Outside the try — see the note on grantAction above.
    redirect(
      `/admin/users/${id}?status=${changed ? 'role_revoked' : 'no_change'}`,
    );
  }

  async function restrictAction(formData: FormData) {
    'use server';
    const s = await getSession();
    if (!s) redirect('/auth/login?next=/admin/users');

    // "Suspend" is not a separate concept server-side -- it is all four
    // capabilities at once. The button just pre-fills them.
    const suspendAll = formData.get('suspend_all') === '1';
    const on = (name: string) =>
      suspendAll || formData.get(name) === 'on';

    const reason = String(formData.get('reason') ?? '').trim();
    // A date input gives a day; send end-of-day UTC so "expires on the
    // 1st" does not lift at midnight as the 1st begins.
    const expiresOn = String(formData.get('expires_on') ?? '').trim();
    const expires_at = expiresOn ? `${expiresOn}T23:59:59Z` : undefined;

    try {
      await setAdminUserRestrictions(s.token, id, {
        ingest_blocked: on('ingest_blocked'),
        sharing_blocked: on('sharing_blocked'),
        public_profile_blocked: on('public_profile_blocked'),
        submissions_blocked: on('submissions_blocked'),
        reason,
        expires_at,
      });
    } catch (e) {
      if (e instanceof ApiCallError) {
        if (e.status === 401) redirect('/auth/login?next=/admin/users');
        if (e.status === 403)
          redirect(`/admin/users/${id}?error=${encodeURIComponent(e.body.error)}`);
        if (e.status === 400)
          redirect(`/admin/users/${id}?error=${encodeURIComponent(e.body.error)}`);
      }
      logger.error({ err: e }, 'admin restrict failed');
      redirect(`/admin/users/${id}?error=unexpected`);
    }
    // Outside the try: redirect() throws NEXT_REDIRECT, so calling it
    // above would be caught by this function's own catch and reroute
    // every SUCCESSFUL restriction to ?error=unexpected.
    redirect(`/admin/users/${id}?status=restricted`);
  }

  async function deleteAccountAction(formData: FormData) {
    'use server';
    const s = await getSession();
    if (!s) redirect('/auth/login?next=/admin/users');

    const confirm_handle = String(formData.get('confirm_handle') ?? '').trim();
    const raw = String(formData.get('mode') ?? 'pseudonymise');
    // Never infer purge from a malformed value -- default to the
    // non-destructive mode and let the server reject if it disagrees.
    const mode = raw === 'purge' ? 'purge' : 'pseudonymise';

    try {
      await deleteAdminUser(s.token, id, { confirm_handle, mode });
    } catch (e) {
      if (e instanceof ApiCallError) {
        if (e.status === 401) redirect('/auth/login?next=/admin/users');
        redirect(
          `/admin/users/${id}?error=${encodeURIComponent(e.body.error)}`,
        );
      }
      logger.error({ err: e }, 'admin delete failed');
      redirect(`/admin/users/${id}?error=unexpected`);
    }
    // Outside the try -- redirect() throws NEXT_REDIRECT, which this
    // function's own catch would otherwise turn into ?error=unexpected
    // on a SUCCESSFUL deletion.
    redirect('/admin/users?status=account_deleted');
  }

  async function reinstateAction() {
    'use server';
    const s = await getSession();
    if (!s) redirect('/auth/login?next=/admin/users');
    try {
      await clearAdminUserRestrictions(s.token, id);
    } catch (e) {
      if (e instanceof ApiCallError && e.status === 401) {
        redirect('/auth/login?next=/admin/users');
      }
      logger.error({ err: e }, 'admin reinstate failed');
      redirect(`/admin/users/${id}?error=unexpected`);
    }
    // Outside the try -- see the note on restrictAction above.
    redirect(`/admin/users/${id}?status=reinstated`);
  }

  return (
    <div
      style={{ display: 'flex', flexDirection: 'column', gap: 20 }}
    >

      <Link
        href={'/admin/users' as Route}
        style={{ fontSize: 13, color: 'var(--accent)', textDecoration: 'none' }}
      >
        ← All users
      </Link>

      <AdminPageHeader
        eyebrow="Admin · user detail"
        title={user.claimed_handle}
        titleClassName="mono"
        lede={
          <>
            {user.email}
            {isSelf && (
              <span style={{ marginLeft: 8, color: 'var(--accent)' }}>
                (you)
              </span>
            )}
          </>
        }
      />

      {status && STATUS_MESSAGES[status] && (
        <div className="ss-badge ss-badge--ok" style={{ alignSelf: 'flex-start' }}>
          {STATUS_MESSAGES[status]}
        </div>
      )}
      {errorCode && (
        <div
          className="ss-badge"
          style={{
            alignSelf: 'flex-start',
            borderColor: 'var(--danger)',
            color: 'var(--danger)',
          }}
        >
          {ERROR_MESSAGES[errorCode] ?? errorCode}
        </div>
      )}

      <section className="ss-card" style={{ padding: '20px 24px' }}>
        <div className="ss-eyebrow" style={{ marginBottom: 6 }}>
          Account
        </div>
        <dl
          style={{
            display: 'grid',
            gridTemplateColumns: 'auto 1fr',
            gap: '8px 16px',
            margin: '10px 0 0',
            fontSize: 13,
          }}
        >
          <Dt>UUID</Dt>
          <Dd>
            <span className="mono">{user.id}</span>
          </Dd>
          <Dt>Joined</Dt>
          <Dd>
            <span className="mono">{user.created_at}</span>
          </Dd>
          <Dt>Email verified</Dt>
          <Dd>{user.email_verified ? '✓ yes' : '· no'}</Dd>
          <Dt>RSI verified</Dt>
          <Dd>{user.rsi_verified ? '✓ yes' : '· no'}</Dd>
          <Dt>2FA enabled</Dt>
          <Dd>{user.totp_enabled ? '✓ yes' : '· no'}</Dd>
          <Dt>Staff roles</Dt>
          <Dd>
            {user.staff_roles.length === 0 ? '—' : user.staff_roles.join(', ')}
          </Dd>
          <Dt>Sync</Dt>
          <Dd>
            <SyncChip state={user.sync_state} />
          </Dd>
          <Dt>Entries</Dt>
          <Dd>
            <span className="mono">{user.entry_count.toLocaleString()}</span>
          </Dd>
          <Dt>Last activity</Dt>
          <Dd>{relativeTime(user.last_activity_at)}</Dd>
        </dl>
      </section>

      <RestrictionPanel
        current={detail.restriction ?? null}
        restrictAction={restrictAction}
        reinstateAction={reinstateAction}
        canModerate={session.staffRoles.some(
          (r) => r === 'admin' || r === 'moderator',
        )}
      />

      <DeleteAccountPanel
        handle={user.claimed_handle}
        deleteAction={deleteAccountAction}
        isAdmin={isAdmin}
      />

      {/* Devices — the per-device breakdown behind the aggregate chip
          above. Both are computed from the SAME rows server-side, so
          the chip can't disagree with the table under it. */}
      <section className="ss-card" style={{ padding: 0, overflow: 'hidden' }}>
        <div style={{ padding: '20px 24px 12px' }}>
          <div className="ss-eyebrow" style={{ marginBottom: 6 }}>
            Devices
          </div>
          <h2 style={sectionHeadingStyle}>Paired devices</h2>
        </div>
        <AdminTable
          columns={DEVICE_COLUMNS}
          rows={devices}
          rowKey={(d) => `${d.label}-${d.last_seen_at ?? 'never'}`}
          empty="No devices have ever been paired for this account."
        />
      </section>

      {/* Activity by type — straight from the stat_event_counts
          rollup, so this is a cheap read rather than a scan. */}
      <section className="ss-card" style={{ padding: 0, overflow: 'hidden' }}>
        <div style={{ padding: '20px 24px 12px' }}>
          <div className="ss-eyebrow" style={{ marginBottom: 6 }}>
            Activity
          </div>
          <h2 style={sectionHeadingStyle}>Entries by event type</h2>
        </div>
        <AdminTable
          columns={EVENT_TYPE_COLUMNS}
          rows={eventTypeCounts}
          rowKey={(c) => c.event_type}
          empty="This account has never sent an event."
        />
      </section>

      {/* Data & retention. Deliberately shows no "swept by retention"
          figure: sweep totals are aggregate and transient, never
          persisted per user, so any number here would be invented. */}
      <section className="ss-card" style={{ padding: '20px 24px' }}>
        <div className="ss-eyebrow" style={{ marginBottom: 6 }}>
          Data
        </div>
        <h2 style={sectionHeadingStyle}>Retention</h2>
        <dl
          style={{
            display: 'grid',
            gridTemplateColumns: 'auto 1fr',
            gap: '8px 16px',
            margin: '10px 0 0',
            fontSize: 13,
          }}
        >
          <Dt>Total entries</Dt>
          <Dd>
            <span className="mono">{user.entry_count.toLocaleString()}</span>
          </Dd>
          <Dt>Oldest retained</Dt>
          <Dd>
            {retention.oldest_entry_at ? (
              <span className="mono">
                {retention.oldest_entry_at.slice(0, 10)}
              </span>
            ) : (
              <span style={{ color: 'var(--fg-dim)' }}>never</span>
            )}
          </Dd>
          <Dt>Retention tier</Dt>
          <Dd>
            {retention.tier}
            {retention.tier === 'free' && (
              <span
                style={{
                  marginLeft: 8,
                  color: 'var(--fg-dim)',
                  fontSize: 12,
                }}
              >
                (a lapsed supporter keeps the pill but reverts to free)
              </span>
            )}
          </Dd>
          <Dt>Retention window</Dt>
          <Dd>{retentionWindow(retention.retention_days)}</Dd>
          <Dt>Subject to purge</Dt>
          <Dd>
            {retention.cutoff ? (
              <>
                yes — events before{' '}
                <span className="mono">{retention.cutoff.slice(0, 10)}</span>
              </>
            ) : (
              'no — retention is unlimited'
            )}
          </Dd>
        </dl>
      </section>

      <section className="ss-card" style={{ padding: '20px 24px' }}>
        <div className="ss-eyebrow" style={{ marginBottom: 6 }}>
          Staff roles
        </div>
        <h2
          style={{
            margin: 0,
            fontSize: 17,
            fontWeight: 600,
            letterSpacing: '-0.01em',
          }}
        >
          Grant / revoke
        </h2>
        {!isAdmin ? (
          <p
            style={{
              margin: '12px 0 0',
              color: 'var(--fg-muted)',
              fontSize: 13,
            }}
          >
            Read-only view. Granting and revoking staff roles requires the
            admin role on your own account.
          </p>
        ) : (
          <div
            style={{
              marginTop: 14,
              display: 'flex',
              flexDirection: 'column',
              gap: 14,
            }}
          >
            <RoleControl
              role="moderator"
              active={isModerator}
              grantAction={grantAction}
              revokeAction={revokeAction}
              disableReason={null}
            />
            <RoleControl
              role="admin"
              active={isAdminTarget}
              grantAction={grantAction}
              revokeAction={revokeAction}
              disableReason={
                isSelf && isAdminTarget
                  ? "You can't revoke your own admin role."
                  : null
              }
            />
          </div>
        )}
      </section>

      {/* Audit v2.1 §C — sharing context sub-tab. Read-only for now;
          one-click admin revoke is in the reports queue. */}
      <SharingContext context={sharing} />
    </div>
  );
}

function SharingContext({ context }: { context: UserSharingContext | null }) {
  return (
    <section
      style={{
        background: 'var(--bg-elev)',
        border: '1px solid var(--border)',
        borderRadius: 0,
        padding: 20,
        display: 'flex',
        flexDirection: 'column',
        gap: 16,
      }}
    >
      <header>
        <h2 style={{ margin: 0 }}>Sharing</h2>
        <p
          style={{
            margin: '4px 0 0',
            color: 'var(--fg-muted)',
            fontSize: 13,
          }}
        >
          Outbound + inbound shares and reports involving this user.
        </p>
      </header>

      {!context ? (
        <p style={{ margin: 0, color: 'var(--fg-muted)', fontSize: 13 }}>
          Couldn&apos;t load sharing context. Try refreshing.
        </p>
      ) : (
        <div
          style={{
            display: 'grid',
            gridTemplateColumns: 'repeat(auto-fit, minmax(260px, 1fr))',
            gap: 16,
          }}
        >
          <SharingBucket
            title="Outbound shares"
            empty="No shares granted by this user."
            edges={context.outbound_shares ?? []}
          />
          <SharingBucket
            title="Inbound shares"
            empty="No shares granted to this user."
            edges={context.inbound_shares ?? []}
          />
          <ReportsBucket
            title="Reports filed"
            empty="No reports filed by this user."
            reports={context.reports_filed ?? []}
            showOwnerColumn={true}
          />
          <ReportsBucket
            title="Reports against"
            empty="No reports filed against this user's shares."
            reports={context.reports_against ?? []}
            showOwnerColumn={false}
          />
        </div>
      )}
    </section>
  );
}

function SharingBucket({
  title,
  empty,
  edges,
}: {
  title: string;
  empty: string;
  edges: UserShareEdge[];
}) {
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
      <h3 style={{ margin: 0, fontSize: 13, fontWeight: 600 }}>{title}</h3>
      {edges.length === 0 ? (
        <p style={{ margin: 0, color: 'var(--fg-muted)', fontSize: 12 }}>
          {empty}
        </p>
      ) : (
        <ul
          style={{
            listStyle: 'none',
            padding: 0,
            margin: 0,
            display: 'flex',
            flexDirection: 'column',
            gap: 4,
            fontSize: 12,
          }}
        >
          {edges.slice(0, 12).map((e) => (
            <li
              key={e.counterparty_handle + '|' + e.created_at}
              style={{
                display: 'flex',
                justifyContent: 'space-between',
                gap: 8,
                padding: '4px 0',
                borderBottom: '1px dotted var(--border)',
              }}
            >
              <span className="mono">{e.counterparty_handle}</span>
              <span style={{ color: 'var(--fg-muted)' }}>
                {e.scope_kind ?? 'full'}
                {e.expires_at
                  ? ` · expires ${new Date(e.expires_at).toLocaleDateString()}`
                  : ''}
              </span>
            </li>
          ))}
          {edges.length > 12 && (
            <li
              style={{
                color: 'var(--fg-dim)',
                fontSize: 11,
                fontStyle: 'italic',
              }}
            >
              +{edges.length - 12} more
            </li>
          )}
        </ul>
      )}
    </div>
  );
}

function ReportsBucket({
  title,
  empty,
  reports,
  showOwnerColumn,
}: {
  title: string;
  empty: string;
  reports: NonNullable<UserSharingContext['reports_filed']>;
  showOwnerColumn: boolean;
}) {
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
      <h3 style={{ margin: 0, fontSize: 13, fontWeight: 600 }}>{title}</h3>
      {reports.length === 0 ? (
        <p style={{ margin: 0, color: 'var(--fg-muted)', fontSize: 12 }}>
          {empty}
        </p>
      ) : (
        <ul
          style={{
            listStyle: 'none',
            padding: 0,
            margin: 0,
            display: 'flex',
            flexDirection: 'column',
            gap: 4,
            fontSize: 12,
          }}
        >
          {reports.slice(0, 8).map((r) => (
            <li
              key={r.id}
              style={{
                display: 'flex',
                justifyContent: 'space-between',
                gap: 8,
                padding: '4px 0',
                borderBottom: '1px dotted var(--border)',
              }}
            >
              <span>
                {r.reason}
                <span
                  style={{ color: 'var(--fg-muted)', marginLeft: 6 }}
                >
                  · {r.status}
                </span>
              </span>
              <span
                className="mono"
                style={{ color: 'var(--fg-muted)' }}
                title={r.created_at}
              >
                {showOwnerColumn ? r.owner_handle : r.recipient_handle}
              </span>
            </li>
          ))}
          {reports.length > 8 && (
            <li
              style={{
                color: 'var(--fg-dim)',
                fontSize: 11,
                fontStyle: 'italic',
              }}
            >
              +{reports.length - 8} more
            </li>
          )}
        </ul>
      )}
    </div>
  );
}

function Dt({ children }: { children: React.ReactNode }) {
  return (
    <dt
      style={{
        color: 'var(--fg-muted)',
        fontSize: 11,
        textTransform: 'uppercase',
        letterSpacing: '0.06em',
        alignSelf: 'center',
      }}
    >
      {children}
    </dt>
  );
}
function Dd({ children }: { children: React.ReactNode }) {
  return <dd style={{ margin: 0 }}>{children}</dd>;
}

function RoleControl({
  role,
  active,
  grantAction,
  revokeAction,
  disableReason,
}: {
  role: 'moderator' | 'admin';
  active: boolean;
  grantAction: (formData: FormData) => Promise<void>;
  revokeAction: (formData: FormData) => Promise<void>;
  disableReason: string | null;
}) {
  return (
    <div
      style={{
        display: 'flex',
        alignItems: 'flex-start',
        justifyContent: 'space-between',
        gap: 16,
        padding: '12px 16px',
        background: 'var(--bg-elev)',
        border: '1px solid var(--border)',
        borderRadius: 0,
        flexWrap: 'wrap',
      }}
    >
      <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
        <span style={{ fontWeight: 600, fontSize: 14 }}>{role}</span>
        <span style={{ color: 'var(--fg-muted)', fontSize: 12 }}>
          {active ? 'Active grant' : 'Not granted'}
        </span>
      </div>
      {disableReason ? (
        <span
          style={{
            color: 'var(--fg-dim)',
            fontSize: 12,
            maxWidth: 280,
            textAlign: 'right',
          }}
        >
          {disableReason}
        </span>
      ) : active ? (
        <form action={revokeAction} style={{ margin: 0 }}>
          <input type="hidden" name="role" value={role} />
          <button
            type="submit"
            className="ss-btn ss-btn--ghost"
            style={{ color: 'var(--danger)' }}
          >
            Revoke {role}
          </button>
        </form>
      ) : (
        <form
          action={grantAction}
          style={{ margin: 0, display: 'flex', gap: 8 }}
        >
          <input type="hidden" name="role" value={role} />
          <input
            type="text"
            name="reason"
            placeholder="Reason (optional)"
            maxLength={280}
            style={{
              padding: '6px 10px',
              background: 'var(--bg)',
              border: '1px solid var(--border)',
              borderRadius: 0,
              color: 'var(--fg)',
              fontSize: 12,
              minWidth: 200,
            }}
          />
          <button type="submit" className="ss-btn ss-btn--primary">
            Grant {role}
          </button>
        </form>
      )}
    </div>
  );
}

const sectionHeadingStyle: React.CSSProperties = {
  margin: 0,
  fontSize: 17,
  fontWeight: 600,
  letterSpacing: '-0.01em',
};

/**
 * A revoked device is shown, not filtered out. An operator debugging
 * "why did their data stop" needs to see that a device WAS paired and
 * then revoked — hiding it makes the account look like it never synced.
 */
const DEVICE_COLUMNS: readonly AdminTableColumn<AdminUserDeviceDto>[] = [
  {
    header: 'Device',
    cell: (d) => <span className="mono">{d.label}</span>,
  },
  {
    header: 'Sync',
    cell: (d) =>
      d.sync_enabled ? (
        <span className="ss-badge ss-badge--ok" style={{ fontSize: 10 }}>
          on
        </span>
      ) : (
        <span
          className="ss-badge"
          style={{ fontSize: 10, color: 'var(--fg-dim)' }}
        >
          off
        </span>
      ),
  },
  {
    header: 'Last seen',
    cell: (d) => (
      <span
        style={{
          color: d.last_seen_at ? 'var(--fg-muted)' : 'var(--fg-dim)',
          fontSize: 12,
        }}
      >
        {relativeTime(d.last_seen_at)}
      </span>
    ),
  },
  {
    header: 'Status',
    cell: (d) =>
      d.revoked_at ? (
        <span style={{ color: 'var(--danger)', fontSize: 12 }}>
          revoked {d.revoked_at.slice(0, 10)}
        </span>
      ) : (
        <span style={{ color: 'var(--fg-muted)', fontSize: 12 }}>active</span>
      ),
  },
];

const EVENT_TYPE_COLUMNS: readonly AdminTableColumn<AdminUserEventTypeCountDto>[] =
  [
    {
      header: 'Event type',
      cell: (c) => <span className="mono">{c.event_type}</span>,
    },
    {
      header: 'Count',
      cell: (c) => (
        <span className="mono">{c.event_count.toLocaleString()}</span>
      ),
    },
    {
      header: 'First seen',
      cell: (c) => (
        <span style={{ color: 'var(--fg-muted)', fontSize: 12 }}>
          {c.first_seen_at ? c.first_seen_at.slice(0, 10) : '—'}
        </span>
      ),
    },
    {
      header: 'Last seen',
      cell: (c) => (
        <span style={{ color: 'var(--fg-muted)', fontSize: 12 }}>
          {relativeTime(c.last_seen_at)}
        </span>
      ),
    },
  ];
