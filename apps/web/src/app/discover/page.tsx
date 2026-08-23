/**
 * Directory — the public-profile listing, in the projection.
 *
 * Server-rendered grid of every StarStats handle whose owner has flipped the
 * SpiceDB public toggle AND not opted out of the listing via `listing_opt_out`.
 * The endpoint is unauthenticated by design — the same data is reachable
 * per-handle at `/v1/public/{handle}/*`, so consolidating it into an index
 * changes nothing about the trust posture.
 *
 * Backend contract:
 *  - GET /v1/discover/profiles?limit=50&after={handle} -> {profiles, next_after}
 *
 * "Load more" pagination stays a client component owning the cursor walk, so
 * the initial server render stays fast and paging needs no hydration-cycle
 * redirect. It portals appended cards into the SSR `<ul>` so both paths share
 * one grid and one card implementation.
 *
 * COVERAGE marks the kit's `Directory.jsx` as inferred, so nothing here is
 * grounded in it — this is a port of the route. The e2e contract is preserved
 * exactly: `discover-page`, `discover-grid`, `discover-profile-card`,
 * `data-handle`, `discover-empty-state`, `discover-load-more`, and the
 * `/u/{handle}?source=discover` href.
 *
 * The session is read for the CHROME only (account menu vs. Sign in). The
 * listing itself never required one and still does not.
 */

import React from 'react';
import Link from 'next/link';
import type { Route } from 'next';
import { Flatline, Plane, type Calibration } from 'holo';
import {
  ApiCallError,
  getDiscoverProfiles,
  type DiscoverProfilesResponse,
} from '@/lib/api';
import { logger } from '@/lib/logger';
import { getSession } from '@/lib/session';
import { getTheme } from '@/lib/theme';
import { navSections } from '@/lib/nav';
import { setCalibrationAction } from '@/app/me/_projection/actions';
import { DiscoverLoadMore } from './_components/DiscoverLoadMore';
import { DiscoverProfileCard } from './_components/DiscoverProfileCard';
import {
  DiscoverProjection,
  type DiscoverSection,
} from './_projection/DiscoverProjection';
import { DIRECTORY_GROUP } from './_projection/groups';

export const metadata = { title: 'Directory' };

// Default request size on the initial render. Mirrors the server-side
// `DEFAULT_LIMIT` so the page sizes align between client and server. If the
// server's default ever changes, this stays truthful (it's just our explicit
// ask) — no contract dependency.
const INITIAL_LIMIT = 50;

export default async function DiscoverPage() {
  const session = await getSession();

  let calibration: Calibration = 'terra';
  try {
    calibration = (await getTheme(session?.token)) as Calibration;
  } catch {
    // Preference read failed; the default stands.
  }

  // Initial fetch happens server-side so the first paint includes the grid.
  // Soft-fail to an empty payload so a transient backend hiccup surfaces as the
  // empty state rather than a hard 5xx — the directory is a casual browse
  // surface, not a load-bearing app route, and the reader cannot act on the
  // failure either way.
  let initial: DiscoverProfilesResponse = { profiles: [], next_after: null };
  try {
    initial = await getDiscoverProfiles({ limit: INITIAL_LIMIT });
  } catch (e) {
    const status = e instanceof ApiCallError ? e.status : undefined;
    logger.error(
      { err: e, call: 'getDiscoverProfiles', status },
      'discover initial fetch rejected',
    );
  }

  const hasProfiles = initial.profiles.length > 0;

  const sections: DiscoverSection[] = [
    {
      id: 'directory',
      title: 'Directory',
      ctx: hasProfiles
        ? `${initial.profiles.length} shown · players who've opened up their profile`
        : "Players who've opened up their profile",
      group: DIRECTORY_GROUP.key,
      node: hasProfiles ? (
        <div data-testid="discover-page">
          {/* A ranked LIST inside a Plane, per `Directory.jsx` — pilots go
              through `CatalogueLayout`'s list state, not the entity grid the
              catalogue uses for ships. The `discover-grid` testid is kept: it
              is how three specs find this element, and renaming it would be
              rewriting tests to match markup rather than the other way round. */}
          <Plane tilt="flat" cap="Public projections" style={{ marginTop: 18 }}>
            <ul data-testid="discover-grid" className="hp-dirlist">
              {initial.profiles.map((p, i) => (
                <li key={p.handle}>
                  <DiscoverProfileCard profile={p} rank={i + 1} />
                </li>
              ))}
            </ul>
          </Plane>
          {initial.next_after ? (
            <DiscoverLoadMore
              initialAfter={initial.next_after}
              limit={INITIAL_LIMIT}
              initialCount={initial.profiles.length}
            />
          ) : null}
        </div>
      ) : (
        <div data-testid="discover-page">
          <div data-testid="discover-empty-state">
            <Flatline
              title="No public profiles to show yet."
              reason="no-data"
              hint={
                <>
                  Make yours public in{' '}
                  <Link href={'/sharing' as Route}>Settings &rarr; Sharing</Link>
                  .
                </>
              }
            />
          </div>
        </div>
      ),
    },
  ];

  return (
    <DiscoverProjection
      handle={session?.claimedHandle}
      calibration={calibration}
      nav={navSections(
        { signedIn: Boolean(session), staffRoles: session?.staffRoles },
        'discover',
      )}
      sections={sections}
      notice={null}
      onCalibrate={async (id: string) => {
        'use server';
        await setCalibrationAction(id);
      }}
    />
  );
}
