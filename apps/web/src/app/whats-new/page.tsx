/**
 * /whats-new — staff news and newly shipped features for the signed-in
 * player, with the same unread state the tray's What's New tab uses. Marking
 * something read here clears it in the tray too, and the other way round.
 *
 * Backend contracts:
 *  - GET  /v1/me/news, POST /v1/me/news/:id/seen
 *  - GET  /v1/me/roadmap/whats-new, POST /v1/me/roadmap/whats-new/seen
 *
 * Bodies are plain text: rendered with line breaks kept, never as HTML.
 */

import Link from 'next/link';
import type { Route } from 'next';
import React from 'react';
import { redirect } from 'next/navigation';
import { BeamAlert, BeamChip, Plane } from 'holo';
import type { Calibration } from 'holo';
import {
  ApiCallError,
  getMyNews,
  getWhatsNew,
  type MyNewsResponse,
  type WhatsNewResponse,
} from '@/lib/api';
import { logger } from '@/lib/logger';
import { navSections } from '@/lib/nav';
import { getSession } from '@/lib/session';
import { getTheme } from '@/lib/theme';
import { setCalibrationAction } from '@/app/me/_projection/actions';
import { formatRelativePast } from '@/app/sharing/_projection/format';
import { ConfirmSubmitButton } from '@/components/forms/ConfirmSubmitButton';
import { PaneSurface, type SurfaceSection } from '@/components/projection/PaneSurface';
import { markAllReadAction, markItemReadAction, markNewsReadAction } from './actions';

export const metadata = { title: "What's new" };

export default async function WhatsNewPage(props: {
  searchParams: Promise<{ status?: string; error?: string }>;
}) {
  const session = await getSession();
  if (!session) redirect('/auth/login?next=/whats-new');
  const params = await props.searchParams;

  let calibration: Calibration = 'terra';
  try {
    calibration = (await getTheme(session.token)) as Calibration;
  } catch (e) {
    logger.warn({ err: e, call: 'whatsnew.theme' }, 'load theme failed');
  }

  const [newsRes, itemsRes] = await Promise.allSettled([
    getMyNews(session.token, 20),
    getWhatsNew(session.token),
  ]);
  for (const [r, call] of [
    [newsRes, 'whatsnew.news'],
    [itemsRes, 'whatsnew.items'],
  ] as const) {
    if (r.status === 'rejected') {
      const status = r.reason instanceof ApiCallError ? r.reason.status : undefined;
      if (status === 401) redirect('/auth/login?next=/whats-new');
      logger.error({ err: r.reason, call, status }, 'whats-new call failed');
    }
  }
  const news: MyNewsResponse | null = newsRes.status === 'fulfilled' ? newsRes.value : null;
  const items: WhatsNewResponse | null = itemsRes.status === 'fulfilled' ? itemsRes.value : null;

  const unreadNews = news?.items.filter((n) => n.unread) ?? [];
  const unreadItems = items?.seen_via_auth ? items.items.filter((i) => i.unread) : [];
  const anyUnread = unreadNews.length + unreadItems.length > 0;

  const unavailable = (what: string) => (
    <BeamAlert tone="bad">Couldn&apos;t load {what}. Refresh to retry.</BeamAlert>
  );

  const sections: SurfaceSection[] = [
    {
      id: 'news',
      title: 'News',
      ctx: news ? `${news.unread_count} unread` : undefined,
      group: 'news',
      node: !news ? (
        unavailable('news')
      ) : news.items.length === 0 ? (
        <p className="hp-prose">No news yet.</p>
      ) : (
        <Plane tilt="flat" style={{ marginTop: 18 }}>
          {news.items.map((n) => (
            <article
              key={n.id}
              className="hp-grant"
              data-testid="whatsnew-news"
              data-unread={n.unread ? 'true' : undefined}
            >
              <div className="hp-grant__who">
                <span>{n.title}</span>
                <span className="hp-grant__note">
                  {formatRelativePast(n.published_at) ?? ''}
                </span>
                {/* Plain text, whitespace kept. Never rendered as HTML. */}
                <p className="hp-prose" style={{ whiteSpace: 'pre-wrap', marginTop: 6 }}>
                  {n.body}
                </p>
                {n.link_url ? (
                  <a href={n.link_url} rel="noopener noreferrer" target="_blank">
                    Read more
                  </a>
                ) : null}
              </div>
              {n.unread ? (
                <>
                  <BeamChip tone="warn">new</BeamChip>
                  <form action={markNewsReadAction}>
                    <input type="hidden" name="id" value={n.id} />
                    <ConfirmSubmitButton className="hp-btn hp-btn--ghost">Mark read</ConfirmSubmitButton>
                  </form>
                </>
              ) : null}
            </article>
          ))}
        </Plane>
      ),
    },
    {
      id: 'shipped',
      title: 'New features',
      ctx: items?.seen_via_auth ? `${unreadItems.length} unread` : undefined,
      group: 'shipped',
      node: !items ? (
        unavailable('new features')
      ) : items.items.length === 0 ? (
        <p className="hp-prose">
          All caught up. The full history is on the{' '}
          <Link href={'/changelog' as Route}>changelog</Link>.
        </p>
      ) : (
        <>
          <Plane tilt="flat" style={{ marginTop: 18 }}>
            {items.items.map((i) => (
              <article
                key={i.roadmap_item_id}
                className="hp-grant"
                data-testid="whatsnew-item"
                data-unread={i.unread ? 'true' : undefined}
              >
                <div className="hp-grant__who">
                  <Link href={`/roadmap/${encodeURIComponent(i.slug)}` as Route}>{i.title}</Link>
                  <span className="hp-grant__note">
                    {i.headline_status} · {formatRelativePast(i.latest_published_at) ?? ''}
                  </span>
                </div>
                {i.unread ? (
                  <>
                    <BeamChip tone="warn">new</BeamChip>
                    <form action={markItemReadAction}>
                      <input type="hidden" name="roadmap_item_id" value={i.roadmap_item_id} />
                      <input
                        type="hidden"
                        name="changelog_entry_id"
                        value={i.latest_changelog_entry_id}
                      />
                      <ConfirmSubmitButton className="hp-btn hp-btn--ghost">Mark read</ConfirmSubmitButton>
                    </form>
                  </>
                ) : null}
              </article>
            ))}
          </Plane>
          <p className="hp-prose">
            The full history is on the <Link href={'/changelog' as Route}>changelog</Link>.
          </p>
        </>
      ),
    },
  ];

  const notice =
    params.status === 'all_read'
      ? { tone: 'good' as const, message: 'All caught up.' }
      : params.error
        ? { tone: 'bad' as const, message: 'Something went wrong. Try again.' }
        : null;

  return (
    <PaneSurface
      handle={session.claimedHandle}
      calibration={calibration}
      nav={navSections({ signedIn: true, staffRoles: session.staffRoles }, 'whats-new')}
      groups={[
        { key: 'news', label: 'News' },
        { key: 'shipped', label: 'New features' },
      ]}
      measure="reading"
      crumb={[{ label: 'Projection', href: '/me' }, { label: "What's new" }]}
      account={[
        { id: 'me', label: 'Projection', href: '/me' },
        { id: 'friends', label: 'Friends', href: '/friends' },
        { id: 'settings', label: 'Calibrate', href: '/settings' },
      ]}
      sections={sections}
      notice={notice}
      banner={
        anyUnread ? (
          <form action={markAllReadAction}>
            {unreadNews.map((n) => (
              <input key={n.id} type="hidden" name="news_id" value={n.id} />
            ))}
            {unreadItems.map((i) => (
              <input
                key={i.roadmap_item_id}
                type="hidden"
                name="item"
                value={`${i.roadmap_item_id}:${i.latest_changelog_entry_id}`}
              />
            ))}
            <ConfirmSubmitButton className="hp-btn">Mark all read</ConfirmSubmitButton>
          </form>
        ) : null
      }
      onCalibrate={async (id: string) => {
        'use server';
        await setCalibrationAction(id);
      }}
    />
  );
}
