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
 * Dense HUD-tile card for a single discover profile.
 *
 * Server-component-safe (no `'use client'` directive) — usable from
 * both the server-rendered `page.tsx` and the client `DiscoverLoadMore`
 * component. Preserves the e2e contract:
 *   - `data-testid="discover-profile-card"`
 *   - `data-handle={handle}`
 *   - href `/u/{encodeURIComponent(handle)}?source=discover`
 */
export function DiscoverProfileCard({ profile: p }: Props) {
  const relative = formatRelative(p.last_active_at);

  return (
    <Link
      href={
        `/u/${encodeURIComponent(p.handle)}?source=discover` as Route
      }
      data-testid="discover-profile-card"
      data-handle={p.handle}
      className="hud-tile"
      style={{
        display: 'flex',
        flexDirection: 'column',
        gap: 6,
        padding: '12px 14px',
        textDecoration: 'none',
        color: 'var(--fg)',
      }}
    >
      <div
        className="mono"
        style={{
          fontSize: 15,
          fontWeight: 600,
          letterSpacing: '-0.01em',
        }}
      >
        {p.handle}
      </div>
      {p.display_name ? (
        <div
          style={{
            color: 'var(--fg-muted)',
            fontSize: 12,
          }}
        >
          {p.display_name}
        </div>
      ) : null}
      {p.supporter ? (
        <div style={{ marginTop: 2 }}>
          <SupporterChip status={p.supporter} size="sm" />
        </div>
      ) : null}
      {relative ? (
        <div
          style={{
            color: 'var(--fg-muted)',
            fontSize: 11,
            marginTop: 'auto',
          }}
        >
          Active {relative}
        </div>
      ) : null}
    </Link>
  );
}
