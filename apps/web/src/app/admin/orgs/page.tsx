import { Plane } from 'holo';
/**
 * Admin · Orgs.
 *
 * System-wide org list with substring search over name/slug. Each
 * row shows the SpiceDB-resolved member count so admins can spot
 * abandoned orgs (0 members, but the row still exists). Clicking a
 * row drills into /admin/orgs/[slug] for the force-delete action.
 */

import Link from 'next/link';
import type { Route } from 'next';
import { redirect } from 'next/navigation';
import {
  ApiCallError,
  getAdminOrgs,
  type AdminOrgDto,
  type AdminOrgListResponse,
} from '@/lib/api';
import { getSession } from '@/lib/session';
import { AdminPageHeader } from '../_components/AdminPageHeader';

interface SearchParams {
  q?: string;
  offset?: string;
}

const PAGE_SIZE = 50;

export default async function AdminOrgsPage(props: {
  searchParams: Promise<SearchParams>;
}) {
  const session = await getSession();
  if (!session) redirect('/auth/login?next=/admin/orgs');

  const params = await props.searchParams;
  const q = params.q?.trim() ?? '';
  const offset = parsePositiveInt(params.offset);

  let result: AdminOrgListResponse;
  try {
    result = await getAdminOrgs(session.token, {
      q: q || undefined,
      limit: PAGE_SIZE,
      offset,
    });
  } catch (e) {
    if (e instanceof ApiCallError && e.status === 401) {
      redirect('/auth/login?next=/admin/orgs');
    }
    if (e instanceof ApiCallError && e.status === 403) {
      redirect('/me');
    }
    throw e;
  }

  const buildHref = (newOffset: number): Route => {
    const qs = new URLSearchParams();
    if (q) qs.set('q', q);
    if (newOffset > 0) qs.set('offset', String(newOffset));
    const s = qs.toString();
    return (s ? `/admin/orgs?${s}` : '/admin/orgs') as Route;
  };

  return (
    <div
      style={{ display: 'flex', flexDirection: 'column', gap: 20 }}
    >

      <AdminPageHeader
        eyebrow="Admin · orgs"
        title="Orgs"
        lede={
          <>
            Every org across the system. Member counts come from SpiceDB
            (owners + admins + members, deduplicated). Force-delete on
            the detail page wipes both the Postgres row and the SpiceDB
            relationship tree.
          </>
        }
      />

      <form
        method="GET"
        action="/admin/orgs"
        style={{ display: 'flex', gap: 8, flexWrap: 'wrap' }}
      >
        <input
          type="search"
          name="q"
          defaultValue={q}
          placeholder="Search name or slug…"
          autoComplete="off"
          spellCheck={false}
          className="mono"
          style={{
            flex: '1 1 260px',
            padding: '8px 12px',
            background: 'var(--bg-elev)',
            border: '1px solid var(--border)',
            borderRadius: 0,
            color: 'var(--fg)',
          }}
        />
        <button type="submit" className="hp-btn hp-btn--primary">
          Search
        </button>
        {q && (
          <Link
            href={'/admin/orgs' as Route}
            className="hp-btn hp-btn--ghost"
            style={{ textDecoration: 'none' }}
          >
            Clear
          </Link>
        )}
      </form>

      <Plane tilt="flat">
        {result.orgs.length === 0 ? (
          <p
            style={{
              margin: 0,
              padding: '40px 24px',
              textAlign: 'center',
              color: 'var(--fg-muted)',
              fontSize: 14,
            }}
          >
            No orgs match this search.
          </p>
        ) : (
          <table
            style={{ width: '100%', borderCollapse: 'collapse', fontSize: 13 }}
          >
            <thead>
              <tr style={{ background: 'var(--bg-elev)' }}>
                <Th>Name</Th>
                <Th>Slug</Th>
                <Th>Created</Th>
                <Th>Members</Th>
              </tr>
            </thead>
            <tbody>
              {result.orgs.map((o) => (
                <OrgRow key={o.slug} org={o} />
              ))}
            </tbody>
          </table>
        )}
      </Plane>

      <nav
        style={{
          display: 'flex',
          justifyContent: 'space-between',
          gap: 12,
          flexWrap: 'wrap',
        }}
      >
        <span className="hp-fine">
          {result.orgs.length === 0
            ? 'Nothing on this page'
            : `Showing ${result.orgs.length} orgs`}
        </span>
        <div style={{ display: 'flex', gap: 8 }}>
          {offset > 0 ? (
            <Link
              href={buildHref(Math.max(0, offset - PAGE_SIZE))}
              className="hp-btn hp-btn--ghost"
            >
              ← Newer
            </Link>
          ) : (
            <span
              className="hp-btn hp-btn--ghost"
              style={{ opacity: 0.4, pointerEvents: 'none' }}
            >
              ← Newer
            </span>
          )}
          {result.has_more ? (
            <Link
              href={buildHref(offset + PAGE_SIZE)}
              className="hp-btn hp-btn--ghost"
            >
              Older →
            </Link>
          ) : (
            <span
              className="hp-btn hp-btn--ghost"
              style={{ opacity: 0.4, pointerEvents: 'none' }}
            >
              Older →
            </span>
          )}
        </div>
      </nav>
    </div>
  );
}

function parsePositiveInt(raw: string | undefined): number {
  if (!raw) return 0;
  const n = Number.parseInt(raw, 10);
  if (!Number.isFinite(n) || n < 0) return 0;
  return n;
}

function Th({ children }: { children: React.ReactNode }) {
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
      }}
    >
      {children}
    </th>
  );
}

function OrgRow({ org }: { org: AdminOrgDto }) {
  const detailHref = (`/admin/orgs/${encodeURIComponent(org.slug)}`) as Route;
  const joined = new Date(org.created_at).toISOString().slice(0, 10);
  return (
    <tr style={{ borderBottom: '1px solid var(--border)' }}>
      <td style={{ padding: '10px 14px' }}>
        <Link
          href={detailHref}
          style={{ color: 'var(--accent)', textDecoration: 'none' }}
        >
          {org.name}
        </Link>
      </td>
      <td style={{ padding: '10px 14px' }}>
        <span className="mono" style={{ color: 'var(--fg-muted)' }}>
          {org.slug}
        </span>
      </td>
      <td style={{ padding: '10px 14px' }}>
        <span
          className="mono hp-kvlabel"
        >
          {joined}
        </span>
      </td>
      <td style={{ padding: '10px 14px' }}>
        <span
          className="hp-chip"
          style={{
            fontSize: 11,
            color: org.member_count === 0 ? 'var(--fg-dim)' : 'var(--fg)',
          }}
        >
          {org.member_count}
        </span>
      </td>
    </tr>
  );
}
