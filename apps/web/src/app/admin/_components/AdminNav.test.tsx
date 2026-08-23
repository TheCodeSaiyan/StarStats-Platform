/**
 * Regression guard for the "clicking the Audit log tab does nothing"
 * bug.
 *
 * Root cause was `prefetch={false}` on the admin tab links. These are
 * dynamic routes, so an auto (default) prefetch resolves only as far
 * as the nearest `loading.tsx` — `admin/loading.tsx` — and caches that
 * skeleton shell. The router needs that cached shell to commit the new
 * URL and paint feedback on click; without it React's transition keeps
 * the previous page on screen for the whole server round trip, so a
 * slow tab reads as a dead click. `admin/loading.tsx` cannot cover for
 * it: on a soft navigation the loading boundary is only reachable via
 * the prefetched shell, so disabling prefetch disables the skeleton
 * too.
 *
 * Why this is a unit test and not an e2e spec: Next.js only prefetches
 * in production builds, and the Playwright harness
 * (`playwright.config.ts`) runs `next dev`. A dev-mode browser issues
 * zero prefetch requests, so no spec in that harness can observe the
 * behaviour either way. Guarding the prop is the only check that
 * actually fails when the bug is reintroduced.
 */

// Explicit React import: this repo's vitest uses the classic JSX
// runtime, so a JSX-rendering component ReferenceErrors without it.
import React from 'react';
import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render } from '@testing-library/react';
import { AdminNav } from './AdminNav';
import { ADMIN_NAV, ADMIN_NAV_ITEMS } from './admin-nav-config';

interface CapturedLink {
  href: string;
  prefetch: unknown;
  label: string;
}

// `vi.hoisted` so the array exists before the hoisted `vi.mock` factory
// runs — a plain top-level `const` would TDZ-crash inside the factory.
const captured = vi.hoisted(() => ({ links: [] as CapturedLink[] }));

// AdminNav derives its active tab from the pathname now that the layout
// owns it, so the route has to be mockable per test.
const nav = vi.hoisted(() => ({ pathname: '/admin' }));

vi.mock('next/navigation', () => ({
  usePathname: () => nav.pathname,
}));

vi.mock('next/link', () => ({
  default: ({
    href,
    prefetch,
    children,
    // Remaining props (aria-current, …) are spread onto the
    // anchor so assertions about active state see what the real Link
    // would render. Without this the mock silently drops them and an
    // active-state test passes or fails for the wrong reason.
    ...rest
  }: {
    href: string;
    prefetch?: boolean;
    children: React.ReactNode;
    [key: string]: unknown;
  }) => {
    captured.links.push({
      href: String(href),
      prefetch,
      // The label is the tab's text node; good enough to name the tab
      // in a failure message.
      label: String(
        (React.Children.toArray(children)[0] as React.ReactElement<{
          children: string;
        }>)?.props?.children ?? '',
      ),
    });
    return (
      <a href={String(href)} {...(rest as Record<string, unknown>)}>
        {children}
      </a>
    );
  },
}));

describe('AdminNav prefetch policy', () => {
  beforeEach(() => {
    // Braces matter: an arrow body that RETURNS a value makes vitest
    // treat it as a teardown callback and call it after each test.
    captured.links.length = 0;
  });

  it('does not disable prefetch on any admin tab', () => {
    render(<AdminNav />);

    expect(captured.links.length).toBeGreaterThan(0);

    const disabled = captured.links.filter((l) => l.prefetch === false);
    expect(
      disabled.map((l) => `${l.label} (${l.href})`),
      'admin tabs must not set prefetch={false} — it strands the ' +
        'admin/loading.tsx skeleton and makes a click look like a no-op',
    ).toEqual([]);
  });

  it('does not disable prefetch on the Audit log tab specifically', () => {
    render(<AdminNav />);

    const audit = captured.links.find((l) => l.href === '/admin/audit');
    expect(audit, 'the Audit log tab should render as a link').toBeDefined();
    // `undefined` (Next's default/auto) is the required value here.
    // `false` is the regression; `true` would be a full-page prefetch
    // of every tab's session-scoped data, which is the cost the
    // original `prefetch={false}` was trying to avoid.
    expect(audit!.prefetch).toBeUndefined();
  });

  it('still renders the Audit log tab pointing at /admin/audit', () => {
    const { getByRole } = render(<AdminNav />);
    expect(getByRole('link', { name: 'Audit log' })).toHaveAttribute(
      'href',
      '/admin/audit',
    );
  });
});

describe('AdminNav grouping', () => {
  beforeEach(() => {
    captured.links.length = 0;
  });

  it('renders every category heading', () => {
    const { getByText } = render(<AdminNav />);
    for (const category of ADMIN_NAV) {
      expect(getByText(category.label)).toBeInTheDocument();
    }
  });

  it('renders exactly one link per configured item', () => {
    const { getAllByRole } = render(<AdminNav />);
    expect(getAllByRole('link')).toHaveLength(ADMIN_NAV_ITEMS.length);
  });

  it('marks only the current item active', () => {
    nav.pathname = '/admin/users';
    const { getByRole } = render(<AdminNav />);
    expect(getByRole('link', { name: 'Users' })).toHaveAttribute(
      'aria-current',
      'page',
    );
    expect(getByRole('link', { name: 'Orgs' })).not.toHaveAttribute('aria-current');
  });
});

describe('AdminNav active state from pathname', () => {
  beforeEach(() => {
    captured.links.length = 0;
    nav.pathname = '/admin';
  });

  it('activates the parent item on a nested route', () => {
    nav.pathname = '/admin/users/abc-123';
    const { getByRole } = render(<AdminNav />);
    expect(getByRole('link', { name: 'Users' })).toHaveAttribute(
      'aria-current',
      'page',
    );
  });

  // `/admin` is a prefix of every admin route, so a naive startsWith
  // lights Dashboard up on every page. This is the test that catches it.
  it('does not mark Dashboard active on a nested admin route', () => {
    nav.pathname = '/admin/users';
    const { getByRole } = render(<AdminNav />);
    expect(getByRole('link', { name: 'Dashboard' })).not.toHaveAttribute('aria-current');
  });

  it('marks Dashboard active on /admin itself', () => {
    nav.pathname = '/admin';
    const { getByRole } = render(<AdminNav />);
    expect(getByRole('link', { name: 'Dashboard' })).toHaveAttribute(
      'aria-current',
      'page',
    );
  });

  // Longest-prefix, not first-match: /admin/sharing/audit must light
  // Sharing, not the unrelated Audit log tab at /admin/audit.
  it('prefers the longest matching prefix', () => {
    nav.pathname = '/admin/sharing/audit';
    const { getByRole } = render(<AdminNav />);
    expect(getByRole('link', { name: 'Sharing' })).toHaveAttribute(
      'aria-current',
      'page',
    );
    expect(getByRole('link', { name: 'Audit log' })).not.toHaveAttribute('aria-current');
  });
});
