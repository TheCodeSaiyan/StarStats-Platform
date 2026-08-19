/**
 * Admin · Org detail.
 *
 * Surfaces the AdminOrgDto + a force-delete confirmation form. The
 * confirmation requires typing the slug exactly to prevent fat-finger
 * deletes — same posture as the owner-facing delete in /orgs/[slug].
 */

import Link from 'next/link';
import { AdminPageHeader } from '../../_components/AdminPageHeader';
import type { Route } from 'next';
import { redirect } from 'next/navigation';
import {
  ApiCallError,
  deleteAdminOrg,
  getAdminOrg,
  getAdminOrgSharingContext,
  type AdminOrgDto,
  type OrgMemberSharingSlice,
  type OrgSharingContext,
} from '@/lib/api';
import { logger } from '@/lib/logger';
import { getSession } from '@/lib/session';

interface PageProps {
  params: Promise<{ slug: string }>;
  searchParams: Promise<{ error?: string }>;
}

const ERROR_MESSAGES: Record<string, string> = {
  slug_mismatch:
    "Slug confirmation didn't match. Type the slug exactly to confirm.",
  forbidden: 'Admin role required.',
  org_not_found: 'Org no longer exists.',
  unexpected: 'Something went wrong. Try again.',
};

export default async function AdminOrgDetailPage(props: PageProps) {
  const session = await getSession();
  if (!session) redirect('/auth/login?next=/admin/orgs');

  const { slug } = await props.params;
  const params = await props.searchParams;
  const errorCode = params.error;

  let org: AdminOrgDto;
  try {
    org = await getAdminOrg(session.token, slug);
  } catch (e) {
    if (e instanceof ApiCallError && e.status === 401) {
      redirect('/auth/login?next=/admin/orgs');
    }
    if (e instanceof ApiCallError && e.status === 403) {
      redirect('/me');
    }
    if (e instanceof ApiCallError && e.status === 404) {
      redirect('/admin/orgs?error=org_not_found');
    }
    throw e;
  }

  // Audit v2.1 §C — per-org sharing context. Fail-soft so a SpiceDB
  // hiccup (503 spicedb_unavailable) or a transient backend error
  // doesn't block the danger-zone form; the sub-tab renders an empty
  // state with a "couldn't load" hint instead.
  let sharing: OrgSharingContext | null = null;
  try {
    sharing = await getAdminOrgSharingContext(session.token, slug);
  } catch (e) {
    logger.warn(
      { err: e, slug },
      'admin org sharing context fetch failed',
    );
  }

  const isAdmin = session.staffRoles.some((r) => r === 'admin');

  async function deleteAction(formData: FormData) {
    'use server';
    const s = await getSession();
    if (!s) redirect('/auth/login?next=/admin/orgs');
    const confirm = String(formData.get('confirm') ?? '').trim();
    if (confirm !== slug) {
      redirect(`/admin/orgs/${encodeURIComponent(slug)}?error=slug_mismatch`);
    }
    try {
      await deleteAdminOrg(s.token, slug);
    } catch (e) {
      if (e instanceof ApiCallError) {
        if (e.status === 401) redirect('/auth/login?next=/admin/orgs');
        if (e.status === 403)
          redirect(`/admin/orgs/${encodeURIComponent(slug)}?error=forbidden`);
        if (e.status === 404) redirect('/admin/orgs?error=org_not_found');
      }
      logger.error({ err: e }, 'admin org delete failed');
      redirect(`/admin/orgs/${encodeURIComponent(slug)}?error=unexpected`);
    }
    // Outside the try: Next implements `redirect()` by throwing a
    // NEXT_REDIRECT sentinel, so calling it above would have been
    // caught below and rerouted every SUCCESSFUL delete to
    // `?error=unexpected`. Same fix as /orgs/new and the auth actions.
    redirect('/admin/orgs?status=org_deleted');
  }

  return (
    <div
      style={{ display: 'flex', flexDirection: 'column', gap: 20 }}
    >

      <Link
        href={'/admin/orgs' as Route}
        style={{ fontSize: 13, color: 'var(--accent)', textDecoration: 'none' }}
      >
        ← All orgs
      </Link>

      <AdminPageHeader
        eyebrow="Admin · org detail"
        title={org.name}
        lede={<span className="mono">{org.slug}</span>}
      />

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
          Org info
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
            <span className="mono">{org.id}</span>
          </Dd>
          <Dt>Slug</Dt>
          <Dd>
            <span className="mono">{org.slug}</span>
          </Dd>
          <Dt>Owner user</Dt>
          <Dd>
            <span className="mono">{org.owner_user_id}</span>
          </Dd>
          <Dt>Created</Dt>
          <Dd>
            <span className="mono">{org.created_at}</span>
          </Dd>
          <Dt>Members</Dt>
          <Dd>{org.member_count}</Dd>
        </dl>
      </section>

      {/* Audit v2.1 §C — per-org sharing context. Aggregate counts
          per member + reports involving members; drilldown to the
          per-user page for full edge detail. */}
      <SharingContext context={sharing} />

      <section
        className="ss-card"
        style={{ padding: '20px 24px', borderColor: 'var(--danger)' }}
      >
        <div className="ss-eyebrow" style={{ marginBottom: 6 }}>
          Danger zone
        </div>
        <h2
          style={{
            margin: 0,
            fontSize: 17,
            fontWeight: 600,
            color: 'var(--danger)',
          }}
        >
          Force-delete org
        </h2>
        <p
          style={{
            margin: '10px 0 14px',
            color: 'var(--fg-muted)',
            fontSize: 13,
          }}
        >
          Wipes the Postgres row AND every SpiceDB relationship
          (members, owner, admins, share-with-org grants). The audit
          log keeps a record. To confirm, type{' '}
          <span className="mono" style={{ color: 'var(--fg)' }}>
            {org.slug}
          </span>{' '}
          into the field below.
        </p>
        {!isAdmin ? (
          <p style={{ color: 'var(--fg-muted)', fontSize: 13 }}>
            Admin role required.
          </p>
        ) : (
          <form
            action={deleteAction}
            style={{ display: 'flex', gap: 8, flexWrap: 'wrap' }}
          >
            <input
              type="text"
              name="confirm"
              placeholder={org.slug}
              required
              autoComplete="off"
              spellCheck={false}
              className="mono"
              style={{
                flex: '1 1 240px',
                padding: '8px 12px',
                background: 'var(--bg-elev)',
                border: '1px solid var(--border)',
                borderRadius: 'var(--r-sm)',
                color: 'var(--fg)',
              }}
            />
            <button
              type="submit"
              className="ss-btn ss-btn--ghost"
              style={{ color: 'var(--danger)', borderColor: 'var(--danger)' }}
            >
              Delete org
            </button>
          </form>
        )}
      </section>
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

function SharingContext({ context }: { context: OrgSharingContext | null }) {
  return (
    <section
      style={{
        background: 'var(--bg-elev)',
        border: '1px solid var(--border)',
        borderRadius: 'var(--r-card)',
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
          Per-member share footprint and reports involving any member.
          Click a handle to drill into per-user detail.
        </p>
      </header>

      {!context ? (
        <p style={{ margin: 0, color: 'var(--fg-muted)', fontSize: 13 }}>
          Couldn&apos;t load sharing context. The SpiceDB sidecar may
          be unavailable.
        </p>
      ) : (
        <div
          style={{
            display: 'grid',
            gridTemplateColumns: 'repeat(auto-fit, minmax(260px, 1fr))',
            gap: 16,
          }}
        >
          <MembersBucket members={context.members ?? []} />
          <ReportsBucket
            title="Reports filed by members"
            empty="No members of this org have filed reports."
            reports={context.reports_filed_by_members ?? []}
            showColumn="owner_handle"
          />
          <ReportsBucket
            title="Reports against members"
            empty="No reports filed against members of this org."
            reports={context.reports_against_members ?? []}
            showColumn="recipient_handle"
          />
        </div>
      )}
    </section>
  );
}

function MembersBucket({ members }: { members: OrgMemberSharingSlice[] }) {
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
      <h3 style={{ margin: 0, fontSize: 13, fontWeight: 600 }}>Members</h3>
      {members.length === 0 ? (
        <p style={{ margin: 0, color: 'var(--fg-muted)', fontSize: 12 }}>
          This org has no SpiceDB members yet.
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
          {members.slice(0, 24).map((m) => (
            <li
              key={`${m.member_role}:${m.member_handle}`}
              style={{
                display: 'flex',
                justifyContent: 'space-between',
                gap: 8,
                padding: '4px 0',
                borderBottom: '1px dotted var(--border)',
              }}
            >
              <Link
                href={
                  `/admin/users/${encodeURIComponent(m.member_handle)}` as Route
                }
                className="mono"
                style={{ color: 'var(--fg)', textDecoration: 'none' }}
              >
                {m.member_handle}
                <span style={{ color: 'var(--fg-muted)', marginLeft: 6 }}>
                  · {m.member_role}
                </span>
              </Link>
              <span style={{ color: 'var(--fg-muted)' }}>
                out {m.outbound_count} · in {m.inbound_count}
              </span>
            </li>
          ))}
          {members.length > 24 && (
            <li
              style={{
                color: 'var(--fg-dim)',
                fontSize: 11,
                fontStyle: 'italic',
              }}
            >
              +{members.length - 24} more
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
  showColumn,
}: {
  title: string;
  empty: string;
  reports: NonNullable<OrgSharingContext['reports_filed_by_members']>;
  showColumn: 'owner_handle' | 'recipient_handle';
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
                <span style={{ color: 'var(--fg-muted)', marginLeft: 6 }}>
                  · {r.status}
                </span>
              </span>
              <span
                className="mono"
                style={{ color: 'var(--fg-muted)' }}
                title={r.created_at}
              >
                {r[showColumn]}
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
