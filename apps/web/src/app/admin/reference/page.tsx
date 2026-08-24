/**
 * Admin · Reference data summary.
 *
 * Surfaces the wiki-sync output (reference_registry) at category
 * granularity: how many rows each category holds, when the sync
 * last touched it. Drill into a category to see the rows.
 *
 * Syncing is manual: the daily poll was removed, so this page holds
 * the only trigger. Without the button here the worker never runs —
 * it waits on a channel nothing else sends to.
 */

import React from 'react';
import { AdminPageHeader } from '../_components/AdminPageHeader';
import Link from 'next/link';
import type { Route } from 'next';
import { redirect } from 'next/navigation';
import {
  ApiCallError,
  getAdminReferenceCategories,
  triggerReferenceSync,
  type AdminReferenceCategoryDto,
} from '@/lib/api';
import { logger } from '@/lib/logger';
import { getSession } from '@/lib/session';

const SYNC_MESSAGES: Record<string, string> = {
  started: 'Reference sync started. Progress is in the server logs.',
  already_running: 'A reference sync is already queued or running.',
  forbidden: 'Moderator role required.',
  unexpected: 'Could not start the sync. Check the server logs.',
};

interface PageProps {
  searchParams: Promise<{ sync?: string }>;
}

const CATEGORY_LABEL: Record<string, string> = {
  vehicle: 'Vehicles',
  weapon: 'Weapons',
  item: 'Items',
  location: 'Locations',
};

const CATEGORY_BLURB: Record<string, string> = {
  vehicle: 'Ships and ground vehicles — the canonical list of class names.',
  weapon: 'FPS + ship-mounted weapons.',
  item: 'Loose items: components, consumables, gear, attachments.',
  location: 'Star systems, planets, moons, stations, jump points.',
};

export default async function AdminReferencePage(props: PageProps) {
  const session = await getSession();
  if (!session) redirect('/auth/login?next=/admin/reference');

  const { sync } = await props.searchParams;
  const syncMessage = sync ? SYNC_MESSAGES[sync] : undefined;

  async function startSync() {
    'use server';
    const s = await getSession();
    if (!s) redirect('/auth/login?next=/admin/reference');

    // `redirect()` signals by throwing NEXT_REDIRECT, so it must not
    // be called inside the try — this catch would swallow it and every
    // successful sync would report `unexpected`. Pick the target here,
    // redirect after.
    //
    // Typed as `Route`, not `string`: `typedRoutes` brands route
    // literals, and those types only exist after `next build` — plain
    // `tsc --noEmit` accepts a bare string here and CI does not.
    let target: Route;
    try {
      const res = await triggerReferenceSync(s.token);
      target = `/admin/reference?sync=${
        res.started ? 'started' : 'already_running'
      }` as Route;
    } catch (e) {
      if (e instanceof ApiCallError && e.status === 401) {
        target = '/auth/login?next=/admin/reference' as Route;
      } else if (e instanceof ApiCallError && e.status === 403) {
        target = '/admin/reference?sync=forbidden' as Route;
      } else {
        logger.error({ err: e }, 'reference sync trigger failed');
        target = '/admin/reference?sync=unexpected' as Route;
      }
    }
    redirect(target);
  }

  let data;
  try {
    data = await getAdminReferenceCategories(session.token);
  } catch (e) {
    if (e instanceof ApiCallError && e.status === 401) {
      redirect('/auth/login?next=/admin/reference');
    }
    if (e instanceof ApiCallError && e.status === 403) {
      redirect('/me');
    }
    throw e;
  }

  return (
    <div
      style={{ display: 'flex', flexDirection: 'column', gap: 20 }}
    >

      <AdminPageHeader
        eyebrow="Admin · reference data"
        title="Reference data"
        lede={
          <>
            Syncing is manual — nothing refreshes this on a schedule. Run
            it after a wiki change or a normalisation rule change, then
            watch the counts and timestamps below.
          </>
        }
      >
        <form action={startSync} style={{ marginTop: 12 }}>
          <button type="submit" className="hp-btn">
            Sync now
          </button>
        </form>

        {syncMessage ? (
          <p
            role="status"
            style={{
              margin: '10px 0 0',
              fontSize: 13,
              color:
                sync === 'started' || sync === 'already_running'
                  ? 'var(--fg-muted)'
                  : 'var(--danger)',
            }}
          >
            {syncMessage}
          </p>
        ) : null}
      </AdminPageHeader>

      <section
        style={{
          display: 'grid',
          gridTemplateColumns: 'repeat(auto-fill, minmax(260px, 1fr))',
          gap: 12,
        }}
      >
        {data.categories.map((c) => (
          <CategoryCard key={c.category} category={c} />
        ))}
      </section>
    </div>
  );
}

function CategoryCard({ category }: { category: AdminReferenceCategoryDto }) {
  const label = CATEGORY_LABEL[category.category] ?? category.category;
  const blurb = CATEGORY_BLURB[category.category] ?? '';
  const href = (`/admin/reference/${encodeURIComponent(category.category)}`) as Route;

  const updated = category.latest_updated_at
    ? new Date(category.latest_updated_at).toLocaleString()
    : 'Never synced';

  const isStale = (() => {
    if (!category.latest_updated_at) return true;
    const ageDays =
      (Date.now() - new Date(category.latest_updated_at).getTime()) /
      (1000 * 60 * 60 * 24);
    return ageDays > 7;
  })();

  return (
    <Link
      href={href}
      className="ss-card"
      style={{
        padding: '18px 20px',
        textDecoration: 'none',
        color: 'inherit',
        display: 'flex',
        flexDirection: 'column',
        gap: 8,
      }}
    >
      <div className="ss-eyebrow">{label}</div>
      <div
        style={{
          display: 'flex',
          justifyContent: 'space-between',
          alignItems: 'baseline',
          gap: 8,
        }}
      >
        <span style={{ fontSize: 26, fontWeight: 600 }}>
          {category.entry_count.toLocaleString()}
        </span>
        <span
          className="hp-chip"
          style={{
            fontSize: 11,
            color: isStale ? 'var(--danger)' : 'var(--fg-muted)',
            borderColor: isStale ? 'var(--danger)' : undefined,
          }}
        >
          {isStale ? 'Stale' : 'Fresh'}
        </span>
      </div>
      <p style={{ margin: 0, fontSize: 12, color: 'var(--fg-muted)' }}>
        {blurb}
      </p>
      <p
        style={{
          margin: '4px 0 0',
          fontSize: 12,
          color: 'var(--fg-muted)',
        }}
      >
        Last sync: <span className="mono">{updated}</span>
      </p>
    </Link>
  );
}
