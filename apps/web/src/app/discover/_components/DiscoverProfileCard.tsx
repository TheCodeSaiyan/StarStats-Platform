import React from 'react';
import Link from 'next/link';
import type { Route } from 'next';
import type { DiscoverProfile } from '@/lib/api';
import { SupporterChip } from '@/components/SupporterChip';

// Relative-time formatter for the "last active" tail. Picks the
// largest unit that fits and rounds down so a 31-day-old timestamp
// reads as "1 month ago" not "31 days ago". Exported so `page.tsx`
// and `DiscoverLoadMore.tsx` share a single definition.
export function formatRelative(iso: string | null | undefined): string | null {
  if (!iso) return null;
  const t = Date.parse(iso);
  if (Number.isNaN(t)) return null;
  const seconds = Math.round((t - Date.now()) / 1000);
  const abs = Math.abs(seconds);
  try {
    const rtf = new Intl.RelativeTimeFormat('en', { numeric: 'auto' });
    if (abs < 60) return rtf.format(seconds, 'second');
    if (abs < 3600) return rtf.format(Math.round(seconds / 60), 'minute');
    if (abs < 86400) return rtf.format(Math.round(seconds / 3600), 'hour');
    if (abs < 2_592_000) return rtf.format(Math.round(seconds / 86400), 'day');
    if (abs < 31_536_000)
      return rtf.format(Math.round(seconds / 2_592_000), 'month');
    return rtf.format(Math.round(seconds / 31_536_000), 'year');
  } catch {
    return new Date(t).toISOString().slice(0, 10);
  }
}

interface Props {
  profile: DiscoverProfile;
}

/**
 * One directory entry — a ranked-list ROW, not a card.
 *
 * `Directory.jsx` draws pilots through `CatalogueLayout`'s list state: numbered
 * rows with a name, a meta line and a trailing value, NOT the entity grid the
 * catalogue uses for ships and weapons. The system makes that distinction
 * deliberately — a person is not a spec sheet — and the first pass at this page
 * missed it because the kit screen was never opened.
 *
 * METERLESS, and that is the honest reading of the spec rather than a
 * shortcut. `CatalogueLayout` documents `meterless: true` for types whose
 * "list rows carry words, not shares", and the kit's own pilot rows are ranked
 * by an event count. The product's listing endpoint returns handle,
 * display_name, last_active_at and supporter — and NOTHING countable. Drawing a
 * share bar would mean inventing the ranking it displays, which is the exact
 * trap the screen's own Unverified banner warns about.
 *
 * Server-component-safe (no `'use client'` directive) — usable from both the
 * server-rendered `page.tsx` and the client `DiscoverLoadMore` component.
 * Preserves the e2e contract exactly:
 *   - `data-testid="discover-profile-card"`
 *   - `data-handle={handle}`
 *   - href `/u/{encodeURIComponent(handle)}?source=discover`
 *
 * The row's shape follows `MeterRow`'s — rank, name, meta, trailing value —
 * without using the component itself, because `MeterRow` renders a `<div>` and
 * these rows must be real links: a directory entry is somewhere you go, and
 * middle-click and open-in-new-tab matter on a list of people.
 */
export function DiscoverProfileCard({
  profile: p,
  rank,
}: Props & { rank?: number }) {
  const relative = formatRelative(p.last_active_at);
  // The meta line carries what the endpoint actually knows: the display name
  // and the supporter standing. Order matches the kit's — recognition first,
  // then the descriptive detail.
  return (
    <Link
      href={`/u/${encodeURIComponent(p.handle)}?source=discover` as Route}
      data-testid="discover-profile-card"
      data-handle={p.handle}
      className="hp-rw hp-rw--text hp-dirrow"
    >
      <span className="rk">
        {typeof rank === 'number' ? String(rank).padStart(2, '0') : ''}
      </span>
      <span className="nm">
        <span className="hp-dirrow__handle">{p.handle}</span>
        {p.supporter ? (
          <SupporterChip status={p.supporter} size="sm" />
        ) : null}
        {p.display_name ? (
          <span className="hp-dirrow__name">{p.display_name}</span>
        ) : null}
      </span>
      {/* The trailing column is the only figure this endpoint carries. Absent
          rather than "unknown" when the profile has never been active. */}
      <span className="vv">{relative ? `Active ${relative}` : '—'}</span>
    </Link>
  );
}
