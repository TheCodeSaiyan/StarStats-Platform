'use client';

/**
 * Admin section navigation — the design system's `Console` index rail.
 *
 * REBUILT ON THE SYSTEM'S COMPONENT. This was a wrapping row of rounded pills;
 * the port's first pass filed the corners off, its second redrew them as
 * hairline tabs — and both were hand-built shells for a surface the system
 * already ships. `Console.prompt.md` is explicit: admin pairs `Console` with
 * `Projection surface="console"`, the rail mirrors the app's real route
 * segments, and "inside a console, planes are flat and rows tighten
 * automatically; don't hand-tune padding."
 *
 * Items come from `admin-nav-config.ts` (single source of truth,
 * filesystem-checked by `admin-nav-config.test.ts`) and keep their grouping.
 *
 * Still a client component: deriving the active item needs the pathname, which
 * a server component in the layout cannot read.
 */

// Explicit React import: this repo's vitest uses the classic JSX runtime, so a
// JSX-rendering component 500s with "React is not defined" without it.
import React from 'react';
import { usePathname } from 'next/navigation';
import { Console, type ConsoleGroup } from 'holo';
import { chromeLink } from '@/components/projection/chromeLink';

import { ADMIN_NAV, ADMIN_NAV_ITEMS } from './admin-nav-config';

/**
 * Longest-prefix match against the nav items.
 *
 * Two cases this has to get right:
 *   - `/admin` is a prefix of every admin route, so Dashboard matches only on
 *     an exact equality, never by prefix.
 *   - `/admin/sharing/audit` is a prefix match for `/admin/sharing`, but
 *     `/admin/audit` is a separate tab, so first-match ordering would be
 *     fragile. Longest wins.
 */
function activeIdFor(pathname: string): string | undefined {
  let best: { id: string; len: number } | undefined;
  for (const item of ADMIN_NAV_ITEMS) {
    const base = item.href.split('?')[0];
    const hit =
      base === '/admin' ? pathname === '/admin' : pathname.startsWith(base);
    if (hit && (!best || base.length > best.len)) {
      best = { id: item.id, len: base.length };
    }
  }
  return best?.id;
}

export function AdminNav({ children }: { children?: React.ReactNode }) {
  const pathname = usePathname();
  const current = activeIdFor(pathname ?? '');

  const groups: ConsoleGroup[] = ADMIN_NAV.map((category) => ({
    title: category.label,
    items: category.items.map((item) => ({
      id: item.id,
      label: item.label,
      href: item.href,
    })),
  }));

  // `renderLink` keeps these as client transitions. Prefetch stays at the
  // DEFAULT (auto) — do not disable it. These are dynamic routes, so an auto
  // prefetch resolves only as far as `admin/loading.tsx` and the router caches
  // that skeleton; on click it can commit the URL and paint immediately, then
  // stream the page in. With `prefetch={false}` there is no cached shell, the
  // transition holds the PREVIOUS page for the whole round trip, and on a slow
  // tab that is indistinguishable from a click that did nothing. That was a
  // real long-standing bug ("clicking the Audit log tab does nothing").
  // `Console` WRAPS the work area — rail plus content, per its prompt — rather
  // than being a strip above it. The children are the admin page.
  return (
    <Console groups={groups} active={current} renderLink={chromeLink}>
      {children}
    </Console>
  );
}
