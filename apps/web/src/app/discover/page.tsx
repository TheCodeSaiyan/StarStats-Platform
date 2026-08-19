/**
 * Discover — public-profile listing surface.
 *
 * Piece 3 of the public-profile UX work. Server-rendered grid of every
 * StarStats handle whose owner has flipped the SpiceDB public toggle
 * AND not opted out of the listing via Piece 4's `listing_opt_out`
 * column. The endpoint is unauthenticated by design — the same data
 * is reachable per-handle at `/v1/public/{handle}/*`, so consolidating
 * it into an index changes nothing about the trust posture.
 *
 * Backend contract:
 *  - GET /v1/discover/profiles?limit=50&after={handle} -> {profiles, next_after}
 *
 * "Load more" pagination is a client component (handles the second-
 * page fetch + concat) so the initial server render stays fast and
 * the cursor walk doesn't require a hydration-cycle redirect.
 */

import React from 'react';
import Link from 'next/link';
import type { Route } from 'next';
import {
  ApiCallError,
  getDiscoverProfiles,
  type DiscoverProfilesResponse,
} from '@/lib/api';
import { logger } from '@/lib/logger';
import { InstrumentStrip } from '@/components/hud/InstrumentStrip';
import { DiscoverLoadMore } from './_components/DiscoverLoadMore';
import { DiscoverProfileCard } from './_components/DiscoverProfileCard';

export const metadata = { title: "Discover" };

// Default request size on the initial render. Mirrors the server-
// side `DEFAULT_LIMIT` const so the page sizes align between client
// and server. If the server's default ever changes, this stays
// truthful (it's just our explicit ask) — no contract dependency.
const INITIAL_LIMIT = 50;

const pageStyle: React.CSSProperties = {
  display: 'flex',
  flexDirection: 'column',
  gap: 20,
  maxWidth: 1100,
  margin: '0 auto',
  padding: '8px 0 60px',
};

const gridStyle: React.CSSProperties = {
  display: 'grid',
  gridTemplateColumns: 'repeat(auto-fit, minmax(240px, 1fr))',
  gap: 16,
};

const emptyStateStyle: React.CSSProperties = {
  padding: '60px 24px',
  textAlign: 'center',
  border: '1px dashed var(--border)',
  borderRadius: 'var(--r-md)',
  color: 'var(--fg-muted)',
  fontSize: 14,
};

export default async function DiscoverPage() {
  // Initial fetch happens server-side so the first paint includes the
  // grid. Soft-fail to an empty payload so a transient backend hiccup
  // surfaces as the empty state rather than a hard 5xx page —
  // /discover is a casual browse surface, not a load-bearing app
  // route.
  let initial: DiscoverProfilesResponse = { profiles: [], next_after: null };
  try {
    initial = await getDiscoverProfiles({ limit: INITIAL_LIMIT });
  } catch (e) {
    const status = e instanceof ApiCallError ? e.status : undefined;
    logger.error(
      { err: e, call: 'getDiscoverProfiles', status },
      'discover initial fetch rejected',
    );
    // initial stays as the empty default — the empty-state copy below
    // tells the user how to get listed; a hard server error gets the
    // same UX as "no public profiles yet" because the user has no
    // way to act on the failure.
  }

  const hasProfiles = initial.profiles.length > 0;

  return (
    <main style={pageStyle} data-testid="discover-page">
      <InstrumentStrip
        title={
          <h1 className="hud-tile__title" style={{ margin: 0, fontSize: 18 }}>
            Discover
          </h1>
        }
        context="Players who've opened up their profile"
        readouts={
          hasProfiles ? [{ k: 'shown', v: initial.profiles.length }] : []
        }
      />

      {hasProfiles ? (
        <>
          <ul
            data-testid="discover-grid"
            style={{
              ...gridStyle,
              listStyle: 'none',
              margin: 0,
              padding: 0,
            }}
          >
            {initial.profiles.map((p) => (
              <li key={p.handle}>
                <DiscoverProfileCard profile={p} />
              </li>
            ))}
          </ul>
          {initial.next_after ? (
            <DiscoverLoadMore
              initialAfter={initial.next_after}
              limit={INITIAL_LIMIT}
            />
          ) : null}
        </>
      ) : (
        <div style={emptyStateStyle} data-testid="discover-empty-state">
          <p style={{ margin: 0, fontWeight: 600, color: 'var(--fg)' }}>
            No public profiles to show yet.
          </p>
          <p style={{ margin: '8px 0 0' }}>
            Make yours public in{' '}
            <Link
              href={'/sharing' as Route}
              style={{ color: 'var(--accent)' }}
            >
              Settings &rarr; Sharing
            </Link>
            .
          </p>
        </div>
      )}
    </main>
  );
}
