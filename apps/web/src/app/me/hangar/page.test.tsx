import React from 'react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen } from '@testing-library/react';

vi.mock('next/navigation', async () => {
  const m = await import('@/test-support/next-navigation');
  return m.navigationMock();
});

vi.mock('next/link', () => ({
  default: ({ href, children }: { href: string; children: React.ReactNode }) => (
    <a href={String(href)}>{children}</a>
  ),
}));

vi.mock('@/lib/session', () => ({ getSession: vi.fn() }));
vi.mock('@/lib/api', () => ({ getMyHangar: vi.fn(), statusOf: vi.fn() }));
vi.mock('@/lib/reference', () => ({ loadAllReferenceBundles: vi.fn() }));
vi.mock('@/lib/theme', () => ({ getTheme: vi.fn() }));
vi.mock('@/app/me/_projection/actions', () => ({
  setCalibrationAction: vi.fn(),
}));

import { getSession } from '@/lib/session';
import { getMyHangar } from '@/lib/api';
import { loadAllReferenceBundles } from '@/lib/reference';
import { getTheme } from '@/lib/theme';
import HangarPage from './page';

const mockSession = getSession as ReturnType<typeof vi.fn>;
const mockHangar = getMyHangar as ReturnType<typeof vi.fn>;
const mockBundles = loadAllReferenceBundles as ReturnType<typeof vi.fn>;
const mockTheme = getTheme as ReturnType<typeof vi.fn>;

/** Empty catalogs — nothing resolves, so every row takes the fallback. */
const emptyCatalogs = {
  catalogs: {
    vehicles: new Map(),
    weapons: new Map(),
    items: new Map(),
  },
};

describe('/me/hangar', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockSession.mockResolvedValue({
      token: 't',
      claimedHandle: 'TestPilot',
      staffRoles: [],
    });
    mockTheme.mockResolvedValue('terra');
    mockBundles.mockResolvedValue(emptyCatalogs);
  });

  it('lists EVERY item, well past the widget cap', async () => {
    // The reason the page exists. The widget stops at 12 and reports
    // "+N more"; a page that also capped would solve nothing. 34 is
    // the count from the report that prompted it.
    mockHangar.mockResolvedValue({
      captured_at: '2026-09-17T12:00:00Z',
      ships: Array.from({ length: 34 }, (_, i) => ({
        name: `Ship ${i}`,
        manufacturer: 'Maker',
      })),
    });
    const node = await HangarPage();
    const { container } = render(node as React.ReactElement);
    const rows = container.querySelectorAll('.hp-snapshots__row');
    expect(rows.length).toBe(34);
    expect(container.textContent).not.toContain('more');
  });

  it('offers no refresh control', async () => {
    // The zero-credentials invariant: the tray owns the RSI session and
    // is the only thing that can re-scrape. A refresh affordance here
    // could only lie about what it does.
    mockHangar.mockResolvedValue({
      captured_at: '2026-09-17T12:00:00Z',
      ships: [{ name: 'Gladius', manufacturer: 'Aegis' }],
    });
    const node = await HangarPage();
    const { container } = render(node as React.ReactElement);
    const controls = Array.from(
      container.querySelectorAll('button, a'),
    ).map((el) => (el.textContent ?? '').toLowerCase());
    expect(controls.some((t) => t.includes('refresh'))).toBe(false);
    // …and it says where refreshing actually happens.
    expect(container.textContent).toContain('tray');
  });

  it('flattens a bundle into its constituent items', async () => {
    // Matches the widget, so the two surfaces never disagree about
    // what counts as an item.
    mockHangar.mockResolvedValue({
      captured_at: '2026-09-17T12:00:00Z',
      ships: [
        {
          name: 'Gear - HighSec - Bundle',
          manufacturer: null,
          contains: ['Alpha Helmet', 'Beta Torso', 'Gamma Legs'],
        },
      ],
    });
    const node = await HangarPage();
    const { container } = render(node as React.ReactElement);
    expect(container.querySelectorAll('.hp-snapshots__row').length).toBe(3);
    expect(container.textContent).toContain('Alpha Helmet');
    expect(container.textContent).toContain('Gamma Legs');
  });

  it('links an unresolved item to a store search', async () => {
    mockHangar.mockResolvedValue({
      captured_at: '2026-09-17T12:00:00Z',
      ships: [
        { name: 'Paints - Railen - Uamchuai Paint', manufacturer: null, kind: 'skin' },
      ],
    });
    const node = await HangarPage();
    const { container } = render(node as React.ReactElement);
    const store = Array.from(container.querySelectorAll('a')).find((a) =>
      (a.getAttribute('href') ?? '').includes('/store/pledge/browse'),
    );
    expect(store, 'unresolved item should link to the store').toBeTruthy();
    expect(store?.getAttribute('href')).toContain('keywords=');
    expect(store?.getAttribute('rel')).toContain('noopener');
  });

  it('tells a missing snapshot apart from a failed fetch', async () => {
    // Both render zero items. They are completely different stories
    // and a shared "nothing here" would send the reader looking in the
    // wrong place.
    mockHangar.mockResolvedValue(null);
    let node = await HangarPage();
    let out = render(node as React.ReactElement);
    expect(out.container.textContent).toContain('No hangar snapshot yet');
    out.unmount();

    mockHangar.mockRejectedValue(new Error('boom'));
    node = await HangarPage();
    out = render(node as React.ReactElement);
    expect(out.container.textContent).toContain("didn't respond");
  });

  it('redirects a signed-out visitor to login', async () => {
    mockSession.mockResolvedValue(null);
    await expect(HangarPage()).rejects.toThrow();
  });
});
