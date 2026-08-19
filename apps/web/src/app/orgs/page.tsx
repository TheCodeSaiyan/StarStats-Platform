import React from 'react';
import Link from 'next/link';
import type { Route } from 'next';
import { redirect } from 'next/navigation';
import {
  ApiCallError,
  listOrgs,
  type ListOrgsResponse,
} from '@/lib/api';
import { logger } from '@/lib/logger';
import { getSession } from '@/lib/session';
import { InstrumentStrip } from '@/components/hud/InstrumentStrip';
import { ControlStrip } from '@/components/hud/ControlStrip';

export const metadata = { title: "Orgs" };

interface SearchParams {
  status?: string;
  error?: string;
  sort?: string;
}

type OrgSort = 'recent' | 'oldest' | 'name';

const ORG_SORT_TABS: ReadonlyArray<{ id: OrgSort; label: string }> = [
  { id: 'recent', label: 'Newest' },
  { id: 'oldest', label: 'Oldest' },
  { id: 'name', label: 'Name' },
];

function parseOrgSort(raw: string | undefined): OrgSort {
  if (raw && ORG_SORT_TABS.some((t) => t.id === raw)) return raw as OrgSort;
  return 'recent';
}

/** Sort the user's orgs client-side — the list is small and fully
 *  fetched, so no API round-trip is needed. `recent` (newest first)
 *  matches the server's default `created_at DESC`. */
function sortOrgs<T extends { name: string; created_at: string }>(
  orgs: readonly T[],
  sort: OrgSort,
): T[] {
  const copy = [...orgs];
  switch (sort) {
    case 'name':
      copy.sort((a, b) => a.name.localeCompare(b.name));
      break;
    case 'oldest':
      copy.sort((a, b) => a.created_at.localeCompare(b.created_at));
      break;
    case 'recent':
    default:
      copy.sort((a, b) => b.created_at.localeCompare(a.created_at));
      break;
  }
  return copy;
}

const mainStyle: React.CSSProperties = {
  maxWidth: 'none',
  margin: 0,
  padding: 0,
  display: 'flex',
  flexDirection: 'column',
  gap: 20,
};

const orgGridStyle: React.CSSProperties = {
  display: 'grid',
  gridTemplateColumns: 'repeat(auto-fit, minmax(280px, 1fr))',
  gap: 16,
};

const orgCardStyle: React.CSSProperties = {
  padding: '12px 14px',
  display: 'flex',
  flexDirection: 'column',
  gap: 8,
};

const orgCardHeadStyle: React.CSSProperties = {
  display: 'flex',
  justifyContent: 'space-between',
  alignItems: 'flex-start',
  gap: 12,
};

const orgNameStyle: React.CSSProperties = {
  margin: 0,
  fontSize: 16,
  fontWeight: 600,
  letterSpacing: '-0.01em',
};

const orgSlugStyle: React.CSSProperties = {
  color: 'var(--fg-dim)',
  fontSize: 12,
};

const orgFootStyle: React.CSSProperties = {
  display: 'flex',
  justifyContent: 'space-between',
  alignItems: 'center',
};

const emptyStyle: React.CSSProperties = {
  textAlign: 'center',
  padding: '40px 20px',
  color: 'var(--fg-muted)',
  fontSize: 14,
};

const emptyTitleStyle: React.CSSProperties = {
  fontSize: 16,
  color: 'var(--fg)',
  marginBottom: 6,
};

export default async function OrgsPage(props: {
  searchParams: Promise<SearchParams>;
}) {
  const session = await getSession();
  if (!session) redirect('/auth/login?next=/orgs');

  const { status, error, sort: sortRaw } = await props.searchParams;
  const sort = parseOrgSort(sortRaw);

  let orgs: ListOrgsResponse = { orgs: [] };
  let degraded = false;
  try {
    orgs = await listOrgs(session.token);
  } catch (e) {
    if (e instanceof ApiCallError && e.status === 401) {
      redirect('/auth/login?next=/orgs');
    }
    if (e instanceof ApiCallError && e.status === 503) {
      degraded = true;
    } else {
      logger.error({ err: e }, 'list orgs failed');
      degraded = true;
    }
  }

  return (
    <main className="ss-screen-enter" style={mainStyle}>
      <InstrumentStrip
        title={<h1 className="hud-tile__title" style={{ margin: 0, fontSize: 18 }}>Your orgs</h1>}
        context="Organizations you own"
        trailing={<Link href="/orgs/new" className="ss-btn ss-btn--primary">+ Create org</Link>}
      />

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

      {degraded ? (
        <section className="ss-card ss-card-pad">
          <div style={emptyStyle}>
            <div style={emptyTitleStyle}>Comms down.</div>
            <div>Organizations are temporarily unavailable. Try again shortly.</div>
          </div>
        </section>
      ) : orgs.orgs.length === 0 ? (
        <section className="ss-card ss-card-pad">
          <div style={emptyStyle}>
            <div style={emptyTitleStyle}>No orgs yet</div>
            <div>
              You don&apos;t own any orgs yet. Create one to share your
              manifest with a group.
            </div>
          </div>
        </section>
      ) : (
        <>
          {orgs.orgs.length > 1 && (
            <nav aria-label="Sort orgs">
              <ControlStrip>
                {ORG_SORT_TABS.map((t) => {
                  const active = t.id === sort;
                  const href = (
                    t.id === 'recent' ? '/orgs' : `/orgs?sort=${t.id}`
                  ) as Route;
                  return (
                    <Link
                      key={t.id}
                      href={href}
                      data-active={active ? 'true' : undefined}
                      style={{
                        background: active ? 'var(--bg-elev)' : 'transparent',
                        border: '1px solid',
                        borderColor: active
                          ? 'var(--border-strong)'
                          : 'transparent',
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
              </ControlStrip>
            </nav>
          )}
          <div style={orgGridStyle}>
            {sortOrgs(orgs.orgs, sort).map((o) => (
            <div key={o.id} className="hud-tile" style={orgCardStyle}>
              <div style={orgCardHeadStyle}>
                <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
                  <h3 style={orgNameStyle}>{o.name}</h3>
                  <span className="mono" style={orgSlugStyle}>
                    /orgs/{o.slug}
                  </span>
                </div>
                <span className="ss-badge ss-badge--accent">Owner</span>
              </div>
              <div style={orgFootStyle}>
                <span style={{ color: 'var(--fg-muted)', fontSize: 13 }}>
                  Owned by you
                </span>
                <Link
                  href={`/orgs/${encodeURIComponent(o.slug)}`}
                  className="ss-btn ss-btn--link"
                >
                  Open →
                </Link>
              </div>
            </div>
            ))}
          </div>
        </>
      )}
    </main>
  );
}

function labelForStatus(code: string): string {
  switch (code) {
    case 'org_created':
      return 'Organization created.';
    case 'org_deleted':
      return 'Organization deleted.';
    default:
      return 'Done.';
  }
}

function labelForError(code: string): string {
  switch (code) {
    case 'slug_collision':
      return "We couldn't generate a unique URL for that name. Try a different name.";
    case 'invalid_name':
      return 'That name is empty or has no usable characters.';
    case 'spicedb_unavailable':
      return 'Organizations are temporarily unavailable. Try again shortly.';
    case 'unexpected':
      return 'Something went wrong. Please try again.';
    default:
      return `Couldn't complete that action (${code}).`;
  }
}
