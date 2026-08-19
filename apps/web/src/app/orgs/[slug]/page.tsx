import Link from 'next/link';
import { redirect } from 'next/navigation';
import {
  ApiCallError,
  addOrgMember,
  deleteOrg,
  getOrg,
  removeOrgMember,
  type GetOrgResponse,
} from '@/lib/api';
import { logger } from '@/lib/logger';
import { getSession } from '@/lib/session';

export const metadata = { title: "Org" };

interface SearchParams {
  status?: string;
  error?: string;
}

const mainStyle: React.CSSProperties = {
  maxWidth: 'none',
  margin: 0,
  padding: 0,
  display: 'flex',
  flexDirection: 'column',
  gap: 20,
};

const headerStyle: React.CSSProperties = {
  display: 'flex',
  alignItems: 'flex-end',
  justifyContent: 'space-between',
  gap: 24,
  flexWrap: 'wrap',
};

const headerTitleStyle: React.CSSProperties = {
  margin: 0,
  fontSize: 32,
  fontWeight: 600,
  letterSpacing: '-0.02em',
};

const headerSubtitleStyle: React.CSSProperties = {
  margin: '6px 0 0',
  color: 'var(--fg-muted)',
  fontSize: 14,
  display: 'flex',
  alignItems: 'center',
  gap: 10,
  flexWrap: 'wrap',
};

const cardHeaderStyle: React.CSSProperties = {
  display: 'flex',
  flexDirection: 'column',
  gap: 4,
  marginBottom: 16,
};

const cardTitleStyle: React.CSSProperties = {
  margin: 0,
  fontSize: 18,
  fontWeight: 600,
  letterSpacing: '-0.01em',
};

const sectionGapStyle: React.CSSProperties = {
  display: 'flex',
  flexDirection: 'column',
  gap: 14,
};

const memberListStyle: React.CSSProperties = {
  listStyle: 'none',
  margin: 0,
  padding: 0,
  display: 'flex',
  flexDirection: 'column',
};

const memberRowStyle: React.CSSProperties = {
  display: 'flex',
  alignItems: 'center',
  justifyContent: 'space-between',
  gap: 12,
  padding: '12px 0',
  borderBottom: '1px solid var(--border)',
};

const formStyle: React.CSSProperties = {
  display: 'flex',
  flexDirection: 'column',
  gap: 14,
  margin: 0,
  maxWidth: 520,
};

const inlineFormGridStyle: React.CSSProperties = {
  display: 'grid',
  gridTemplateColumns: '1fr 160px',
  gap: 12,
};

const formActionsStyle: React.CSSProperties = {
  display: 'flex',
  marginTop: 4,
};

const dangerCardStyle: React.CSSProperties = {
  borderColor: 'color-mix(in oklab, var(--danger) 40%, transparent)',
  background: 'color-mix(in oklab, var(--danger) 6%, var(--surface))',
};

const emptyStyle: React.CSSProperties = {
  textAlign: 'center',
  padding: '32px 16px',
  color: 'var(--fg-muted)',
  fontSize: 14,
};

const emptyTitleStyle: React.CSSProperties = {
  fontSize: 16,
  color: 'var(--fg)',
  marginBottom: 6,
};

export default async function OrgDetailPage(props: {
  params: Promise<{ slug: string }>;
  searchParams: Promise<SearchParams>;
}) {
  const session = await getSession();
  const { slug } = await props.params;
  if (!session) redirect(`/auth/login?next=/orgs/${encodeURIComponent(slug)}`);

  const { status, error } = await props.searchParams;

  let org: GetOrgResponse | null = null;
  let notFound = false;
  let degraded = false;
  try {
    org = await getOrg(session.token, slug);
  } catch (e) {
    if (e instanceof ApiCallError && e.status === 401) {
      redirect(`/auth/login?next=/orgs/${encodeURIComponent(slug)}`);
    }
    if (e instanceof ApiCallError && e.status === 404) {
      notFound = true;
    } else if (e instanceof ApiCallError && e.status === 503) {
      degraded = true;
    } else {
      logger.error({ err: e, slug }, 'load org failed');
      degraded = true;
    }
  }

  async function addMemberAction(formData: FormData) {
    'use server';
    const session = await getSession();
    if (!session) {
      redirect(`/auth/login?next=/orgs/${encodeURIComponent(slug)}`);
    }
    const handle = String(formData.get('handle') ?? '').trim();
    const roleRaw = String(formData.get('role') ?? '');
    const role: 'admin' | 'member' = roleRaw === 'admin' ? 'admin' : 'member';
    if (handle === '') {
      redirect(
        `/orgs/${encodeURIComponent(slug)}?error=invalid_handle`,
      );
    }
    try {
      await addOrgMember(session.token, slug, { handle, role });
    } catch (e) {
      if (e instanceof ApiCallError) {
        if (e.status === 401) {
          redirect(`/auth/login?next=/orgs/${encodeURIComponent(slug)}`);
        }
        if (e.status === 404) {
          redirect(
            `/orgs/${encodeURIComponent(slug)}?error=recipient_not_found`,
          );
        }
        if (e.status === 400) {
          redirect(
            `/orgs/${encodeURIComponent(slug)}?error=${encodeURIComponent(e.body.error)}`,
          );
        }
        if (e.status === 403) {
          redirect(`/orgs/${encodeURIComponent(slug)}?error=forbidden`);
        }
        if (e.status === 503) {
          redirect(
            `/orgs/${encodeURIComponent(slug)}?error=spicedb_unavailable`,
          );
        }
      }
      logger.error({ err: e, slug }, 'add org member failed');
      redirect(`/orgs/${encodeURIComponent(slug)}?error=unexpected`);
    }
    redirect(`/orgs/${encodeURIComponent(slug)}?status=member_added`);
  }

  async function removeMemberAction(formData: FormData) {
    'use server';
    const session = await getSession();
    if (!session) {
      redirect(`/auth/login?next=/orgs/${encodeURIComponent(slug)}`);
    }
    const handle = String(formData.get('handle') ?? '').trim();
    if (handle === '') return;
    try {
      await removeOrgMember(session.token, slug, handle);
    } catch (e) {
      if (e instanceof ApiCallError) {
        if (e.status === 401) {
          redirect(`/auth/login?next=/orgs/${encodeURIComponent(slug)}`);
        }
        if (e.status === 403) {
          redirect(`/orgs/${encodeURIComponent(slug)}?error=forbidden`);
        }
      }
      logger.error({ err: e, slug, handle }, 'remove org member failed');
      redirect(`/orgs/${encodeURIComponent(slug)}?error=unexpected`);
    }
    redirect(`/orgs/${encodeURIComponent(slug)}?status=member_removed`);
  }

  async function deleteOrgAction() {
    'use server';
    const session = await getSession();
    if (!session) {
      redirect(`/auth/login?next=/orgs/${encodeURIComponent(slug)}`);
    }
    try {
      await deleteOrg(session.token, slug);
    } catch (e) {
      if (e instanceof ApiCallError) {
        if (e.status === 401) {
          redirect(`/auth/login?next=/orgs/${encodeURIComponent(slug)}`);
        }
        if (e.status === 403) {
          redirect(`/orgs/${encodeURIComponent(slug)}?error=forbidden`);
        }
      }
      logger.error({ err: e, slug }, 'delete org failed');
      redirect(`/orgs/${encodeURIComponent(slug)}?error=unexpected`);
    }
    redirect('/orgs?status=org_deleted');
  }

  if (notFound) {
    return (
      <main className="ss-screen-enter" style={mainStyle}>
        <header>
          <div className="ss-eyebrow" style={{ marginBottom: 8 }}>
            Orgs
          </div>
          <h1 style={headerTitleStyle}>Org not found</h1>
          <p style={{ ...headerSubtitleStyle, display: 'block' }}>
            Either this org doesn&apos;t exist, or you aren&apos;t a member.
          </p>
        </header>
        <section className="ss-card ss-card-pad">
          <Link href="/orgs" className="ss-btn ss-btn--ghost">
            ← Back to your orgs
          </Link>
        </section>
      </main>
    );
  }

  if (degraded || !org) {
    return (
      <main className="ss-screen-enter" style={mainStyle}>
        <header>
          <div className="ss-eyebrow" style={{ marginBottom: 8 }}>
            Orgs
          </div>
          <h1 style={headerTitleStyle}>Org unavailable</h1>
          <p style={{ ...headerSubtitleStyle, display: 'block' }}>
            The authorisation service is offline. Try again shortly.
          </p>
        </header>
      </main>
    );
  }

  const canManageMembers =
    org.your_role === 'owner' || org.your_role === 'admin';
  const isOwner = org.your_role === 'owner';
  const memberCount = org.members.length;
  const roleBadgeKind =
    org.your_role === 'owner'
      ? 'ss-badge--accent'
      : org.your_role === 'admin'
        ? 'ss-badge--warn'
        : '';

  return (
    <main className="ss-screen-enter" style={mainStyle}>
      <header style={headerStyle}>
        <div>
          <div className="ss-eyebrow" style={{ marginBottom: 8 }}>
            Org
          </div>
          <h1 style={headerTitleStyle}>{org.org.name}</h1>
          <div style={headerSubtitleStyle}>
            <span className="mono">/orgs/{org.org.slug}</span>
            {org.your_role && (
              <span className={`ss-badge ${roleBadgeKind}`}>
                {org.your_role}
              </span>
            )}
          </div>
        </div>
        <Link href="/orgs" className="ss-btn ss-btn--ghost">
          ← All orgs
        </Link>
      </header>

      {status && (
        <div className="ss-alert ss-alert--ok" role="status">
          {labelForStatus(status)}
        </div>
      )}
      {error && (
        <div className="ss-alert ss-alert--danger" role="alert">
          {labelForError(error)}
        </div>
      )}

      <section className="ss-card ss-card-pad">
        <div style={cardHeaderStyle}>
          <span className="ss-eyebrow">Details</span>
          <h2 style={cardTitleStyle}>Org info</h2>
        </div>
        <dl className="ss-kv">
          <dt>Org SID</dt>
          <dd>
            <span className="mono">{org.org.slug}</span>
          </dd>
          <dt>Name</dt>
          <dd>{org.org.name}</dd>
          <dt>Your role</dt>
          <dd>
            <span className={`ss-badge ${roleBadgeKind}`}>
              {org.your_role ?? 'unknown'}
            </span>
          </dd>
          <dt>Members</dt>
          <dd>
            <span className="mono">{memberCount}</span>
          </dd>
        </dl>
      </section>

      <section className="ss-card ss-card-pad">
        <div style={cardHeaderStyle}>
          <span className="ss-eyebrow">Crew</span>
          <h2 style={cardTitleStyle}>Members ({memberCount})</h2>
        </div>
        {memberCount === 0 ? (
          <div style={emptyStyle}>
            <div style={emptyTitleStyle}>Scope is clear.</div>
            <div>No members yet.</div>
          </div>
        ) : (
          <ul style={memberListStyle}>
            {org.members.map((m, idx) => {
              const badgeKind =
                m.role === 'owner'
                  ? 'ss-badge--accent'
                  : m.role === 'admin'
                    ? 'ss-badge--warn'
                    : '';
              const isLast = idx === org!.members.length - 1;
              return (
                <li
                  key={`${m.handle}:${m.role}`}
                  style={
                    isLast
                      ? { ...memberRowStyle, borderBottom: 'none' }
                      : memberRowStyle
                  }
                >
                  <div
                    style={{
                      display: 'flex',
                      alignItems: 'center',
                      gap: 10,
                    }}
                  >
                    <span className="mono">{m.handle}</span>
                    <span className={`ss-badge ${badgeKind}`}>{m.role}</span>
                  </div>
                  {canManageMembers && m.role !== 'owner' && (
                    <form
                      action={removeMemberAction}
                      style={{ margin: 0 }}
                    >
                      <input
                        type="hidden"
                        name="handle"
                        value={m.handle}
                      />
                      <button
                        type="submit"
                        className="ss-btn ss-btn--danger"
                        style={{ padding: '6px 12px', fontSize: 12 }}
                      >
                        Remove
                      </button>
                    </form>
                  )}
                </li>
              );
            })}
          </ul>
        )}
      </section>

      {canManageMembers && (
        <section className="ss-card ss-card-pad">
          <div style={cardHeaderStyle}>
            <span className="ss-eyebrow">Add member</span>
            <h2 style={cardTitleStyle}>Bring someone aboard</h2>
          </div>
          <form action={addMemberAction} style={formStyle}>
            <div style={inlineFormGridStyle}>
              <label className="ss-label">
                <span className="ss-label-text">RSI handle</span>
                <input
                  className="ss-input"
                  type="text"
                  name="handle"
                  required
                  autoComplete="off"
                  spellCheck={false}
                  placeholder="TheCodeSaiyan"
                />
              </label>
              <label className="ss-label">
                <span className="ss-label-text">Role</span>
                <select
                  className="ss-input"
                  name="role"
                  defaultValue="member"
                >
                  <option value="member">Member</option>
                  <option value="admin">Admin</option>
                </select>
              </label>
            </div>
            <div style={formActionsStyle}>
              <button type="submit" className="ss-btn ss-btn--primary">
                Add to org
              </button>
            </div>
          </form>
        </section>
      )}

      {isOwner && (
        <section className="ss-card ss-card-pad" style={dangerCardStyle}>
          <div style={cardHeaderStyle}>
            <span className="ss-eyebrow" style={{ color: 'var(--danger)' }}>
              Danger zone
            </span>
            <h2 style={{ ...cardTitleStyle, color: 'var(--danger)' }}>
              Decommission this org
            </h2>
          </div>
          <div style={sectionGapStyle}>
            <p style={{ margin: 0, color: 'var(--fg-muted)', fontSize: 14 }}>
              Deleting the org removes every membership row. Members lose
              access immediately. The slug{' '}
              <span className="mono">{org.org.slug}</span> is permanent —
              recreating it later requires a fresh slug.
            </p>
            <form action={deleteOrgAction} style={{ margin: 0 }}>
              <button type="submit" className="ss-btn ss-btn--danger">
                Delete org
              </button>
            </form>
          </div>
        </section>
      )}
    </main>
  );
}

function labelForStatus(code: string): string {
  switch (code) {
    case 'member_added':
      return 'Member added.';
    case 'member_removed':
      return 'Member removed.';
    default:
      return 'Done.';
  }
}

function labelForError(code: string): string {
  switch (code) {
    case 'recipient_not_found':
      return "We couldn't find a StarStats user with that handle.";
    case 'invalid_handle':
      return 'That handle looks invalid. Use letters, digits, _ or -.';
    case 'invalid_role':
      return 'Role must be admin or member.';
    case 'forbidden':
      return "You don't have permission to do that.";
    case 'spicedb_unavailable':
      return 'Organizations are temporarily unavailable. Try again shortly.';
    case 'unexpected':
      return 'Something went wrong. Please try again.';
    default:
      return `Couldn't complete that action (${code}).`;
  }
}
