'use client';

import React, { useEffect } from 'react';
import Link from 'next/link';
import type { Route } from 'next';
import { usePathname } from 'next/navigation';
import type { ResolvedLocation, SupporterStatusDto } from '@/lib/api';
import { telemetryFrames } from './telemetry';

interface NavItem {
  label: string;
  href: Route;
  match?: (pathname: string) => boolean;
  /**
   * Optional count rendered as a small badge after the label. Used
   * for surfacing inbound state (e.g. # of people sharing with you)
   * so the user notices without having to visit the page. Hidden
   * when undefined or 0 so the rail stays calm for users with no
   * activity.
   */
  badge?: number;
}

interface NavSection {
  title: string;
  items: NavItem[];
}

interface Props {
  handle: string | null;
  /**
   * Site-wide staff grants for the current user (e.g. `["moderator"]`,
   * `["admin"]`). Mirrored from `/v1/auth/me` into the session cookie
   * at sign-in. Kept for layout API compatibility — the Admin link now
   * lives in AccountMenu (TopBar), not the rail itself.
   */
  staffRoles: string[];
  /**
   * Number of inbound shares (people who have shared their manifest
   * with the current user). Kept for layout API compatibility — the
   * badge on "Shared with me" now lives in AccountMenu (TopBar).
   * Optional — undefined or 0 has no effect.
   */
  inboundShareCount?: number;
  /**
   * Current in-game location + supporter status, forwarded from the
   * layout's existing shell fetches (no extra round-trip). Drives the
   * signature telemetry rail — a persistent bracketed live-readout strip
   * under the nav. Both fail-soft: null hides that frame.
   */
  location?: ResolvedLocation | null;
  supporter?: SupporterStatusDto | null;
  /** Lifetime headline figures streamed as bracketed frames in the rail. */
  eventsTotal?: number | null;
  locationsCount?: number | null;
}

function buildNav(handle: string | null): NavSection[] {
  const profileHref = (handle ? `/u/${encodeURIComponent(handle)}` : '/settings') as Route;
  void profileHref; // profile now lives in the @handle menu; pillars below
  return [
    {
      title: 'Insights',
      items: [
        { label: 'Me', href: '/me' as Route, match: (p) => p === '/me' },
        {
          label: 'Discover',
          href: '/discover' as Route,
          match: (p) => p === '/discover' || p.startsWith('/discover/'),
        },
        {
          label: 'Orgs',
          href: '/orgs' as Route,
          match: (p) => p === '/orgs' || p.startsWith('/orgs/'),
        },
      ],
    },
  ];
}

function isActive(item: NavItem, pathname: string): boolean {
  if (item.match) return item.match(pathname);
  return pathname === item.href;
}

export function LeftRail({
  handle,
  staffRoles: _staffRoles,
  inboundShareCount: _inboundShareCount = 0,
  location = null,
  supporter = null,
  eventsTotal = null,
  locationsCount = null,
}: Props) {
  const pathname = usePathname() ?? '';
  const sections = buildNav(handle);
  const frames = telemetryFrames({
    location,
    supporter,
    eventsTotal,
    locationsCount,
  });

  // Mobile drawer leaks across soft-navigations: clicking a nav link
  // closes the rail visually but leaves `body[data-drawer="open"]` set,
  // so the next page mounts with a "stuck open" rail. Clear on every
  // pathname change. Runs server-side too but `document` is gated.
  useEffect(() => {
    if (typeof document !== 'undefined') {
      delete document.body.dataset.drawer;
    }
  }, [pathname]);

  return (
    <aside className="ss-rail" aria-label="Primary navigation">
      {sections.map((section) => (
        <div key={section.title}>
          <div className="ss-rail-section">{section.title}</div>
          {section.items.map((item) => {
            const active = isActive(item, pathname);
            return (
              <Link
                key={item.href + item.label}
                href={item.href}
                className="ss-rail-item"
                data-active={active ? 'true' : undefined}
                style={{ textDecoration: 'none' }}
              >
                <span className="ss-rail-dot" aria-hidden="true" />
                <span style={{ flex: 1 }}>{item.label}</span>
                {item.badge !== undefined && item.badge > 0 && (
                  <span
                    className="ss-rail-badge"
                    aria-label={`${item.badge} new`}
                  >
                    {item.badge > 99 ? '99+' : item.badge}
                  </span>
                )}
              </Link>
            );
          })}
        </div>
      ))}

      {/* Signature telemetry rail — persistent live-readout frames under the
          nav, each earning corner brackets (.hud-tile--live). Fed by the
          layout's existing location/supporter fetches, so no extra
          round-trip. Frames fail-soft: absent data hides the frame. */}
      {frames.length > 0 && (
        <div className="ss-rail-telemetry">
          <div className="ss-rail-section">Telemetry</div>
          {frames.map((f) => (
            <div key={f.label} className="ss-rail-frame hud-tile--live">
              <div className="ss-rail-frame__label ss-placard">{f.label}</div>
              <div
                className="ss-rail-frame__value mono"
                style={f.accent ? { color: 'var(--accent)' } : undefined}
              >
                {f.value}
              </div>
            </div>
          ))}
        </div>
      )}
    </aside>
  );
}
