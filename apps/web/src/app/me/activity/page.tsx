/**
 * `/me/activity` — the owner-only event log.
 *
 * The "see all" behind the `/me` projection's Recent activity digest. Where
 * the digest folds runs and hides instrumentation to fit eight rows, this
 * page owes the reader every line: one row per event, newest first, each
 * rendered as the sentence its payload says (the same formatter the digest
 * and the tray use) with the raw type as the tooltip.
 *
 * Controls, all URL params so a view is shareable:
 *   - `?range=`  — the window, as on `/me/travel` (`parseRange`).
 *   - `?type=`   — one event type, passed to the server's exact filter.
 *   - `?all=1`   — include the low-signal types the digest hides. Off by
 *                  default; the page says how many rows it is holding back.
 *   - `?before=` — older-page cursor (`before_seq`). "Older →" appears only
 *                  when the page came back full, "← Newest" whenever a
 *                  cursor is set.
 *
 * Owner-only: `/v1/me/events` is me-scoped with no friend equivalent — same
 * gate as `/me/contracts`. Signed-out → login redirect.
 *
 * The type chips are counted from the FETCHED page, not the whole window: a
 * per-type aggregate would be a second query for a figure that only steers a
 * filter. Same call the tray's Logs pane made.
 */

import 'server-only';
import React from 'react';
import type { Route } from 'next';
import Link from 'next/link';
import { redirect } from 'next/navigation';
import { Plane, LogRow, Flatline, BeamAlert, type Calibration } from 'holo';
import { RecordsIndex } from '@/components/projection/RecordsIndex';
import { getSession } from '@/lib/session';
import { listEvents, statusOf, type ListEventsResponse } from '@/lib/api';
import { logger } from '@/lib/logger';
import { parseRange, rangeToSinceIso, rangeLabel } from '@/lib/range';
import {
  PAGE_SIZE,
  parseType,
  parseSeq,
  filterQuery,
  hrefFor,
  type ActivityQuery,
} from './_lib/query';
import { loadAllReferenceBundles } from '@/lib/reference';
import { formatEventType } from '@/lib/event-types';
import { navSections } from '@/lib/nav';
import { getTheme } from '@/lib/theme';
import { setCalibrationAction } from '@/app/me/_projection/actions';
import {
  recentActivityRows,
  type RecentRefs,
} from '@/app/me/_projection/recent-activity-rows';
import { isLowSignal } from '@/app/_components/widgets/recent_activity';
import {
  ActivityProjection,
  type ActivitySection,
} from './_projection/ActivityProjection';

export const metadata = { title: 'Activity' };

interface PageProps {
  searchParams?: Promise<{
    range?: string;
    type?: string;
    before?: string;
    all?: string;
  }>;
}

export default async function ActivityPage(props: PageProps) {
  const session = await getSession();
  if (!session) redirect('/auth/login?next=/me/activity');

  const token = session.token;
  const sp = props.searchParams ? await props.searchParams : {};
  const q: ActivityQuery = {
    range: parseRange(sp.range),
    type: parseType(sp.type),
    all: sp.all === '1',
    before: parseSeq(sp.before),
  };

  let calibration: Calibration = 'terra';
  try {
    calibration = (await getTheme(token)) as Calibration;
  } catch {
    // Preference read failed; the default stands.
  }

  let resp: ListEventsResponse | null = null;
  try {
    resp = await listEvents(token, {
      limit: PAGE_SIZE,
      since: rangeToSinceIso(q.range),
      event_type: q.type,
      before_seq: q.before,
    });
  } catch (err) {
    logger.warn(
      { err, call: 'me.activity.events', status: statusOf(err) },
      'fetch failed',
    );
  }

  // Same degrade as the projection: no catalogue, plain text, never a worse
  // row than the raw value.
  let refs: RecentRefs | undefined;
  try {
    const bundle = await loadAllReferenceBundles();
    refs = { lookup: bundle.lookup, catalogs: bundle.catalogs };
  } catch (err) {
    logger.warn({ err, call: 'me.activity.catalogs' }, 'catalogue load failed');
  }

  const fetched = resp?.events ?? [];
  // A type filter is an explicit ask — never hold back rows the reader named.
  const visible =
    q.all || q.type ? fetched : fetched.filter((e) => !isLowSignal(e));
  const hiddenCount = fetched.length - visible.length;
  const rows = recentActivityRows(visible, refs, {
    now: new Date(),
    fold: false,
    clock: true,
  });

  // Older-page cursor: the smallest seq on THIS page. Offered only when the
  // page came back full, because a short page is the end of the window.
  const oldestSeq = fetched.reduce<number | undefined>(
    (min, e) => (min === undefined || e.seq < min ? e.seq : min),
    undefined,
  );
  const hasOlder = fetched.length >= PAGE_SIZE && oldestSeq !== undefined;

  // Type chips, counted from the fetched page (see module doc).
  const typeCounts = new Map<string, number>();
  for (const e of fetched) {
    typeCounts.set(e.event_type, (typeCounts.get(e.event_type) ?? 0) + 1);
  }
  const types = [...typeCounts.entries()].sort((a, b) => b[1] - a[1]);

  const chip = (
    href: Route,
    label: React.ReactNode,
    active: boolean,
    n?: number,
  ) => (
    <Link
      key={String(href)}
      href={href}
      prefetch={false}
      className="hp-catchip"
      data-active={active ? 'true' : undefined}
      aria-current={active ? 'page' : undefined}
    >
      {label}
      {typeof n === 'number' ? (
        <b className="hp-catchip__n">{n.toLocaleString('en-GB')}</b>
      ) : null}
    </Link>
  );

  const sections: ActivitySection[] =
    resp === null
      ? []
      : [
          {
            id: 'filters',
            title: 'Filter',
            ctx: q.type ? formatEventType(q.type).label : 'every type',
            group: 'log',
            node: (
              <>
                <RecordsIndex active="/me/activity" />
                <nav className="hp-catstrip" aria-label="Event types">
                  {chip(
                    hrefFor({ ...q, type: undefined, before: undefined }),
                    'All',
                    !q.type,
                  )}
                  {types.map(([t, n]) =>
                    chip(
                      hrefFor({ ...q, type: t, before: undefined }),
                      <span title={t}>{formatEventType(t).label}</span>,
                      q.type === t,
                      n,
                    ),
                  )}
                </nav>
                <p className="hp-prose">
                  {q.type ? (
                    'Showing every event of this type.'
                  ) : q.all ? (
                    <>
                      Showing everything, instrumentation included.{' '}
                      <Link href={hrefFor({ ...q, all: false, before: undefined })}>
                        Hide instrumentation
                      </Link>
                    </>
                  ) : (
                    <>
                      Instrumentation is hidden
                      {hiddenCount > 0
                        ? ` · ${hiddenCount.toLocaleString('en-GB')} hidden on this page`
                        : ''}
                      .{' '}
                      <Link href={hrefFor({ ...q, all: true, before: undefined })}>
                        Show everything
                      </Link>
                    </>
                  )}
                </p>
              </>
            ),
          },
          {
            id: 'timeline',
            title: 'Timeline',
            ctx: `${rows.length.toLocaleString('en-GB')} shown · ${rangeLabel(q.range)}`,
            group: 'log',
            node: (
              <>
                {rows.length === 0 ? (
                  <Flatline reason="no-data" />
                ) : (
                  <Plane tilt="flat" cap="Events" hint="newest first">
                    {rows.map((r) => (
                      <LogRow
                        key={r.key}
                        time={r.time}
                        event={r.event}
                        tone={r.tone}
                        mark={r.mark}
                      />
                    ))}
                  </Plane>
                )}
                <nav
                  aria-label="Pages"
                  style={{ display: 'flex', gap: 8, marginTop: 16 }}
                >
                  {q.before !== undefined ? (
                    <Link
                      href={hrefFor({ ...q, before: undefined })}
                      className="hp-btn hp-btn--ghost"
                    >
                      ← Newest
                    </Link>
                  ) : null}
                  {hasOlder ? (
                    <Link
                      href={hrefFor({ ...q, before: oldestSeq })}
                      className="hp-btn hp-btn--ghost"
                    >
                      Older →
                    </Link>
                  ) : null}
                </nav>
              </>
            ),
          },
        ];

  return (
    <ActivityProjection
      handle={session.claimedHandle}
      calibration={calibration}
      range={q.range}
      filterQuery={filterQuery(q)}
      nav={navSections({ signedIn: true, staffRoles: session.staffRoles }, 'me')}
      sections={sections}
      notice={null}
      banner={
        resp === null ? (
          <BeamAlert tone="bad">
            Couldn&apos;t load the event log — the events service didn&apos;t
            respond. Try reloading.
          </BeamAlert>
        ) : null
      }
      onCalibrate={async (id: string) => {
        'use server';
        await setCalibrationAction(id);
      }}
    />
  );
}
