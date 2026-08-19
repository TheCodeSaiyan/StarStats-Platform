'use client';

/**
 * Pagination button for /discover.
 *
 * Server renders the first page; this component owns the cursor walk
 * for subsequent pages. Each "Load more" click:
 *   1. POSTs nothing — calls the GET proxy at `/api/discover/profiles`
 *      with the most recent `next_after` cursor.
 *   2. Appends the returned profiles to the live grid via a React portal
 *      into the SSR `<ul data-testid="discover-grid">`.
 *   3. Updates the cursor; when the upstream returns `next_after = null`
 *      the button hides itself.
 *
 * Appended cards use `<DiscoverProfileCard>` so both the SSR path and
 * the load-more path share a single card implementation.
 */

import React from 'react';
import { createPortal } from 'react-dom';
import { useCallback, useEffect, useState } from 'react';
import type { DiscoverProfile } from '@/lib/api';
import { DiscoverProfileCard } from './DiscoverProfileCard';

interface ProxyResponse {
  profiles: DiscoverProfile[];
  next_after: string | null;
}

interface Props {
  initialAfter: string;
  limit: number;
}

export function DiscoverLoadMore({ initialAfter, limit }: Props) {
  const [cursor, setCursor] = useState<string | null>(initialAfter);
  const [extra, setExtra] = useState<DiscoverProfile[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [gridEl, setGridEl] = useState<HTMLUListElement | null>(null);

  // Locate the SSR grid once on mount so we can portal cards into it.
  useEffect(() => {
    const el = document.querySelector<HTMLUListElement>(
      '[data-testid="discover-grid"]',
    );
    setGridEl(el);
  }, []);

  const onClick = useCallback(async () => {
    if (cursor === null || loading) return;
    setLoading(true);
    setError(null);
    try {
      const params = new URLSearchParams();
      params.set('after', cursor);
      params.set('limit', String(limit));
      const resp = await fetch(`/api/discover/profiles?${params.toString()}`, {
        method: 'GET',
        cache: 'no-store',
      });
      if (!resp.ok) {
        throw new Error(`http_${resp.status}`);
      }
      const body = (await resp.json()) as ProxyResponse;
      setExtra((prev) => [...prev, ...body.profiles]);
      setCursor(body.next_after);
    } catch (e) {
      setError(e instanceof Error ? e.message : 'unknown_error');
    } finally {
      setLoading(false);
    }
  }, [cursor, limit, loading]);

  // Cursor exhausted AND we never hit an error this session -> hide.
  // Keep the button visible if an error landed so the user can retry.
  if (cursor === null && error === null) return null;

  return (
    <>
      {/* Portal appended cards into the SSR <ul> so they share the
          same grid container and data-testid="discover-profile-card"
          remains queryable regardless of SSR vs. load-more source. */}
      {gridEl && extra.length > 0
        ? createPortal(
            extra.map((p) => (
              <li key={p.handle}>
                <DiscoverProfileCard profile={p} />
              </li>
            )),
            gridEl,
          )
        : null}
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'center',
          gap: 12,
          paddingTop: 12,
        }}
      >
        <button
          type="button"
          className="ss-btn ss-btn--ghost"
          onClick={onClick}
          disabled={loading || cursor === null}
          data-testid="discover-load-more"
        >
          {loading ? 'Loading…' : cursor === null ? 'All loaded' : 'Load more'}
        </button>
        {error ? (
          <span
            style={{ color: 'var(--danger)', fontSize: 12 }}
            role="alert"
          >
            Couldn&apos;t load more. Try again.
          </span>
        ) : null}
      </div>
    </>
  );
}
