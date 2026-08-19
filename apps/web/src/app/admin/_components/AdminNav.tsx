'use client';

/**
 * Admin section sub-navigation.
 *
 * Items come from `admin-nav-config.ts` (single source of truth,
 * filesystem-checked by `admin-nav-config.test.ts`) and render grouped
 * by category rather than as one flat wrapping row.
 *
 * Rendered once by `admin/layout.tsx`, not by each page. That is why
 * this is a client component: deriving the active tab needs the current
 * pathname, which a server component in the layout cannot read.
 */

// Explicit React import: this repo's vitest uses the classic JSX
// runtime, so a JSX-rendering component 500s with "React is not
// defined" under test without it (the prod Next build uses the
// automatic runtime and doesn't need it).
import React from 'react';
import Link from 'next/link';
import type { Route } from 'next';

import { usePathname } from 'next/navigation';

import { ADMIN_NAV, ADMIN_NAV_ITEMS } from './admin-nav-config';

/**
 * Longest-prefix match against the nav items.
 *
 * Two cases this has to get right:
 *   - `/admin` is a prefix of every admin route, so Dashboard matches
 *     only on an exact equality, never by prefix.
 *   - `/admin/sharing/audit` is a prefix match for BOTH `/admin/sharing`
 *     and (textually) nothing else — but `/admin/audit` is a separate
 *     tab, so first-match ordering would be fragile. Longest wins.
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

export function AdminNav() {
  const pathname = usePathname();
  const current = activeIdFor(pathname ?? '');

  return (
    <nav
      aria-label="Admin sections"
      style={{
        display: 'flex',
        flexWrap: 'wrap',
        gap: 14,
        borderBottom: '1px solid var(--border)',
        paddingBottom: 12,
        marginBottom: 4,
      }}
    >
      {ADMIN_NAV.map((category) => (
        <div
          key={category.key}
          style={{ display: 'flex', flexDirection: 'column', gap: 4 }}
        >
          {/* Sanctioned ss-eyebrow use: a section category label above
              the items it labels, not per-card decoration. */}
          <span className="ss-eyebrow" style={{ fontSize: 10, paddingLeft: 6 }}>
            {category.label}
          </span>
          <div style={{ display: 'flex', flexWrap: 'wrap', gap: 4 }}>
            {category.items.map((item) => {
              const active = item.id === current;

              // Prefetch is deliberately LEFT AT THE DEFAULT (auto). Do not
              // re-add `prefetch={false}` here — it is what caused the
              // long-standing "clicking the Audit log tab does nothing" bug.
              //
              // Mechanism: these are dynamic routes, so an auto prefetch
              // resolves only as far as the nearest `loading.tsx` — that is
              // `admin/loading.tsx`, the skeleton. The router caches that
              // shell. On click it can therefore commit the new URL and
              // paint the skeleton immediately, then stream the real page in
              // behind it.
              //
              // With `prefetch={false}` there is no cached shell, so the
              // router has nothing to commit and React's transition holds
              // the *previous* page on screen for the entire server round
              // trip: no URL change, no skeleton, no spinner. On a slow tab
              // that is indistinguishable from a click that did nothing.
              // `admin/loading.tsx` cannot help — a loading boundary is only
              // reachable on soft nav via the prefetched shell it lives in,
              // so disabling prefetch silently disables the skeleton too.
              //
              // The original cost argument for `prefetch={false}` ("each tab
              // fetches session-scoped data") predates `admin/loading.tsx`
              // and no longer holds. Next's documented behaviour: "Static
              // routes are prefetched in full by default, whereas dynamic
              // routes are only prefetched if they contain a loading.js
              // boundary" — and the prefetch stops AT that boundary, with
              // the rest of the tree deferred until the real navigation.
              // These routes are dynamic (session-scoped), so an auto
              // prefetch resolves the skeleton and none of the pages' data
              // fetches. Stated from the documented contract, NOT from a
              // measurement of live request counts.
              //
              // NB: Next.js only prefetches in production builds, so none of
              // this is observable under `next dev` (which is what the
              // Playwright e2e harness runs). `AdminNav.test.tsx` guards the
              // prop instead.
              return (
                <Link
                  key={item.id}
                  href={item.href as Route}
                  data-active={active ? 'true' : undefined}
                  style={{
                    padding: '6px 12px',
                    borderRadius: 'var(--r-pill)',
                    fontSize: 13,
                    textDecoration: 'none',
                    display: 'inline-flex',
                    alignItems: 'center',
                    border: '1px solid',
                    background: active ? 'var(--bg-elev)' : 'transparent',
                    borderColor: active ? 'var(--border-strong)' : 'transparent',
                    color: active ? 'var(--fg)' : 'var(--fg-muted)',
                  }}
                >
                  <span>{item.label}</span>
                </Link>
              );
            })}
          </div>
        </div>
      ))}
    </nav>
  );
}
