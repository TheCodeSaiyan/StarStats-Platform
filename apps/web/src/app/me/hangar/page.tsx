/**
 * `/me/hangar` — every item in the pledge ledger.
 *
 * Exists because there was nowhere to see them. The `hangar` widget
 * caps at 12 rows and reports the remainder as "+N more"; the settings
 * pane shows a shorter preview joined by separators. An account with
 * 34 items could see 12 of them and no further, on any surface.
 *
 * The widget's note gave the reason as "hangar has no detail page
 * (zero-credentials invariant)". That invariant governs REFRESH: per
 * CLAUDE.md, anything offering to refresh hangar-style data must point
 * at the tray rather than imply a server-side fetch from RSI. Reading
 * back a snapshot the server already stores fetches nothing, so it
 * does not conflict — and `/me/loadout` is the same shape already.
 * Accordingly this page HAS NO REFRESH CONTROL; it reports when the
 * snapshot was taken and says which component owns re-scraping.
 *
 * Owner-only: `/v1/me/hangar` is me-scoped with no friend equivalent,
 * the same gate as `/me/loadout` and `/me/contracts`. Signed-out goes
 * to login.
 *
 * Not range-aware. A hangar snapshot is a point in time rather than a
 * window over events, so there is no range for a `<RangeBar>` to pick.
 *
 * Bundles are flattened the same way the widget flattens them: a
 * pledge whose `contains` lists more than one real item renders one
 * row per constituent, with the bundle named alongside, because a
 * single opaque "Gear - HighSec - Bundle" row tells the reader
 * nothing about what they own.
 */

import 'server-only';
import React from 'react';
import { redirect } from 'next/navigation';
import { RecordsIndex } from '@/components/projection/RecordsIndex';
import { getSession } from '@/lib/session';
import { getMyHangar, statusOf } from '@/lib/api';
import { loadAllReferenceBundles } from '@/lib/reference';
import { logger } from '@/lib/logger';
import { prettyHangarItem, classifyContainedItem } from '@/lib/hangar-label';
import { fmtNum } from '@/app/_components/widgets/kit/format';
import { type Calibration } from 'holo';
import { navSections } from '@/lib/nav';
import { getTheme } from '@/lib/theme';
import { setCalibrationAction } from '@/app/me/_projection/actions';
import { HangarProjection, type HangarSection } from './_projection/HangarProjection';
import { HangarItemRow, type HangarRow } from './_projection/HangarItemRow';

export const metadata = { title: 'Hangar' };

export default async function HangarPage() {
  const session = await getSession();
  if (!session) redirect('/auth/login?next=/me/hangar');

  const token = session.token;

  let calibration: Calibration = 'terra';
  try {
    calibration = (await getTheme(token)) as Calibration;
  } catch {
    // Preference read failed; the default stands.
  }

  // 404 from this endpoint is "no snapshot yet" (the server holds no
  // RSI credentials and cannot make one), which is a different story
  // from "the request failed" and is told differently below.
  let snapshot: Awaited<ReturnType<typeof getMyHangar>> | null = null;
  let failed = false;
  try {
    snapshot = await getMyHangar(token);
  } catch (err) {
    failed = true;
    logger.warn(
      { err, call: 'me.hangar', status: statusOf(err) },
      'fetch failed',
    );
  }

  // Catalogs resolve item names to KB pages. A failure here degrades
  // every row to its store-search fallback rather than failing the
  // page — the list of what you own is the point, the links are not.
  let catalogs: Awaited<ReturnType<typeof loadAllReferenceBundles>>['catalogs'] | null =
    null;
  try {
    catalogs = (await loadAllReferenceBundles()).catalogs;
  } catch (err) {
    logger.warn({ err, call: 'me.hangar.reference' }, 'catalog load failed');
  }

  const ships = snapshot?.ships ?? [];

  // Flatten exactly as the widget does, so the two surfaces never
  // disagree about what counts as an item.
  const rows: HangarRow[] = [];
  ships.forEach((s, i) => {
    const contains = s.contains ?? [];
    if (contains.length > 1) {
      contains.forEach((raw, j) => {
        const item = raw.trim();
        if (!item) return;
        rows.push({
          key: `${s.name}-${i}-c${j}`,
          label: item,
          category: classifyContainedItem(item),
          note: s.name,
        });
      });
      return;
    }
    const { label, category } = prettyHangarItem(s.name, s.kind);
    rows.push({
      key: `${s.name}-${i}`,
      label,
      category,
      note: s.kind ?? null,
    });
  });

  const sections: HangarSection[] = [
    {
      id: 'summary',
      title: 'Hangar',
      ctx: snapshot ? `captured ${snapshot.captured_at}` : undefined,
      group: 'hangar',
      node: (
        <>
          <RecordsIndex active="/me/hangar" />
          <p className="hp-prose">
            {failed
              ? "Couldn't load your hangar — the service didn't respond. Try reloading."
              : !snapshot
                ? 'No hangar snapshot yet. The StarStats tray reads your pledge ledger from RSI and uploads it; the server never holds your RSI credentials, so it cannot fetch one on its own.'
                : `${fmtNum(rows.length)} item${rows.length === 1 ? '' : 's'} from the snapshot the tray last uploaded.`}
          </p>
          {snapshot ? (
            <p className="hp-note">
              Refreshing happens in the tray, which owns the RSI session — this
              page only reads what it last sent.
            </p>
          ) : null}
        </>
      ),
    },
    {
      id: 'items',
      title: 'Items',
      ctx: rows.length > 0 ? `${fmtNum(rows.length)} total` : undefined,
      group: 'hangar',
      node:
        rows.length === 0 ? (
          <p className="hp-note">Nothing to list yet.</p>
        ) : (
          // Every row, uncapped. That is the entire reason this page
          // exists, so a cap here would defeat it; the widget keeps its
          // 12-row limit because a tile has to fit.
          <ul className="hp-snapshots">
            {rows.map((row) => (
              <HangarItemRow key={row.key} row={row} catalogs={catalogs} />
            ))}
          </ul>
        ),
    },
  ];

  return (
    <HangarProjection
      handle={session.claimedHandle}
      calibration={calibration}
      nav={navSections(
        { signedIn: true, staffRoles: session.staffRoles },
        'hangar',
      )}
      sections={sections}
      notice={null}
      banner={null}
      onCalibrate={async (id: string) => {
        'use server';
        await setCalibrationAction(id);
      }}
    />
  );
}
