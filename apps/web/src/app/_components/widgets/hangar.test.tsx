import React from 'react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render } from '@testing-library/react';

// next/link → plain <a> so the KB deep-link path renders in jsdom.
vi.mock('next/link', () => ({
  default: ({ href, children }: { href: string; children: React.ReactNode }) => (
    <a href={String(href)}>{children}</a>
  ),
}));

vi.mock('@/lib/api', () => ({
  getMyHangar: vi.fn(),
}));

// Catalog fetch is stubbed empty by default (no KB links) so the
// zero-credentials "no link" assertions hold; individual tests override
// it to exercise the deep-link path.
vi.mock('@/lib/reference', () => ({
  loadAllReferenceBundles: vi.fn().mockResolvedValue({
    catalogs: {
      vehicles: new Map(),
      weapons: new Map(),
      items: new Map(),
      locations: new Map(),
    },
  }),
}));

import { getMyHangar } from '@/lib/api';
import { loadAllReferenceBundles } from '@/lib/reference';
import type { ReferenceEntry } from '@/lib/reference-types';
import { hangarWidget } from './hangar';
import { DEFAULT_SHARE_SCOPES, type ViewerCtx } from './types';

function ownerCtx(isOwner = true): ViewerCtx {
  return {
    ownerHandle: 'alice',
    viewerHandle: isOwner ? 'alice' : 'bob',
    isOwner,
    token: 'tok',
    shareScopes: { ...DEFAULT_SHARE_SCOPES },
    recipientScopes: null,
    range: '30d',
  };
}

const mockHangar = () => getMyHangar as ReturnType<typeof vi.fn>;

describe('hangarWidget', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('is not range-aware and owner-only', async () => {
    expect(hangarWidget.rangeAware ?? false).toBe(false);
    expect(await hangarWidget.isAvailable(ownerCtx(true))).toBe(true);
    expect(await hangarWidget.isAvailable(ownerCtx(false))).toBe(false);
  });

  it('compact shows the ship count and the zero-credentials tray caveat', async () => {
    mockHangar().mockResolvedValue({
      captured_at: '2026-05-22T12:00:00Z',
      ships: [
        { name: 'Gladius', manufacturer: 'Aegis' },
        { name: 'Cutlass Black', manufacturer: 'Drake' },
      ],
    });
    const node = await hangarWidget.render(ownerCtx(), 'compact');
    const { container } = render(node as React.ReactElement);
    expect(container.textContent).toContain('2');
    expect(container.textContent).toContain('server holds no RSI credentials');
    // Zero-credentials invariant: NO server-side refresh affordance, no link.
    expect(container.querySelector('a')).toBeNull();
  });

  it('expanded caps the ship list at 12 and surfaces the rest as "+N more" (never scrolls, never links)', async () => {
    mockHangar().mockResolvedValue({
      captured_at: '2026-05-22T12:00:00Z',
      ships: Array.from({ length: 15 }, (_, i) => ({
        name: `Ship ${i}`,
        manufacturer: 'Maker',
      })),
    });
    const node = await hangarWidget.render(ownerCtx(), 'expanded');
    const { container } = render(node as React.ReactElement);
    const rows = container.querySelectorAll('.hud-readout-row');
    expect(rows.length).toBe(12);
    expect(container.textContent).toContain('+3 more');
    // No see-more link — hangar has no detail page.
    expect(container.querySelector('a')).toBeNull();
  });

  it('expanded shows no "+N more" note when every ship already fits', async () => {
    mockHangar().mockResolvedValue({
      captured_at: '2026-05-22T12:00:00Z',
      ships: [{ name: 'Gladius', manufacturer: 'Aegis' }],
    });
    const node = await hangarWidget.render(ownerCtx(), 'expanded');
    const { container } = render(node as React.ReactElement);
    expect(container.textContent).not.toContain('more');
  });

  it('returns null when there are no ships', async () => {
    mockHangar().mockResolvedValue({ captured_at: '2026-05-22T12:00:00Z', ships: [] });
    expect(await hangarWidget.render(ownerCtx(), 'compact')).toBeNull();
  });

  it('returns null when the snapshot is absent (404 → null)', async () => {
    mockHangar().mockResolvedValue(null);
    expect(await hangarWidget.render(ownerCtx(), 'compact')).toBeNull();
  });

  it('returns null when the fetch rejects', async () => {
    mockHangar().mockRejectedValue(new Error('boom'));
    expect(await hangarWidget.render(ownerCtx(), 'compact')).toBeNull();
  });

  it('prettifies the raw pledge name and deep-links a ship to the KB', async () => {
    const railen: ReferenceEntry = {
      category: 'vehicle',
      class_name: 'Railen',
      display_name: 'Railen',
      slug: 'railen',
      summary: { category: 'vehicle' },
    };
    (loadAllReferenceBundles as ReturnType<typeof vi.fn>).mockResolvedValueOnce({
      catalogs: { vehicles: new Map([['railen', railen]]), weapons: new Map() },
    });
    mockHangar().mockResolvedValue({
      captured_at: '2026-05-22T12:00:00Z',
      ships: [{ name: 'Standalone Ships - Railen', manufacturer: 'Aegis', kind: 'ship' }],
    });
    const node = await hangarWidget.render(ownerCtx(), 'expanded');
    const { container } = render(node as React.ReactElement);
    // Prefix stripped: the raw "Standalone Ships - " is gone.
    expect(container.textContent).toContain('Railen');
    expect(container.textContent).not.toContain('Standalone Ships');
    // Ship deep-links to its KB page.
    const link = container.querySelector('a[href="/kb/vehicle/railen"]');
    expect(link).not.toBeNull();
  });

  it('never echoes the item name in the value column (tray-derived manufacturer duplicates the label)', async () => {
    mockHangar().mockResolvedValue({
      captured_at: '2026-05-22T12:00:00Z',
      ships: [
        // The tray derives `manufacturer` by splitting the pledge name on
        // " - ", so it's just the item name again — it must NOT surface
        // as the row value (the "Railen | Railen" duplication bug).
        { name: 'Standalone Ships - Railen', manufacturer: 'Railen', kind: 'ship' },
        {
          name: 'Subscribers Store - Salvaged Skull Relax to the Max Set',
          manufacturer: 'Salvaged Skull Relax to the Max Set',
          kind: 'flair',
        },
        // A paint keeps a USEFUL value: the ship it's for (middle segment),
        // which differs from the paint-name label.
        { name: 'Paints - Railen - Uamchuai Paint', manufacturer: 'Uamchuai Paint', kind: 'skin' },
      ],
    });
    const node = await hangarWidget.render(ownerCtx(), 'expanded');
    const { container } = render(node as React.ReactElement);
    const rows = container.querySelectorAll('.hud-readout-row');
    expect(rows.length).toBe(3);
    rows.forEach((row) => {
      const label = row.querySelector('.hud-trunc')?.textContent?.trim() ?? '';
      const value = row.querySelector('.hud-readout')?.textContent?.trim() ?? '';
      expect(label.length).toBeGreaterThan(0);
      // No row shows the same string on the left (label) and right (value).
      if (value.length > 0) expect(value).not.toBe(label);
    });
    // The two non-paint rows drop the pseudo-manufacturer entirely.
    const values = Array.from(rows).map(
      (r) => r.querySelector('.hud-readout')?.textContent?.trim() ?? '',
    );
    expect(values.filter((v) => v.length > 0)).toEqual(['Railen']); // only the paint's ship
  });

  it('renders a paint as plain text (no link) with the ship in the value column', async () => {
    mockHangar().mockResolvedValue({
      captured_at: '2026-05-22T12:00:00Z',
      ships: [
        { name: 'Paints - Railen - Uamchuai Paint', manufacturer: null, kind: 'skin' },
      ],
    });
    const node = await hangarWidget.render(ownerCtx(), 'expanded');
    const { container } = render(node as React.ReactElement);
    expect(container.textContent).toContain('Uamchuai Paint');
    // The ship it's for surfaces as the concise value.
    expect(container.textContent).toContain('Railen');
    // Cosmetic → never a KB link.
    expect(container.querySelector('a')).toBeNull();
  });
});

describe('hangarWidget bundle expansion', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('expands a bundle (contains.length > 1) into one row per constituent item', async () => {
    mockHangar().mockResolvedValue({
      captured_at: '2026-07-22T00:00:00Z',
      ships: [
        {
          name: 'Gear - HighSec - Bundle',
          kind: 'Gear',
          contains: ['Aegis Avenger Titan', 'Alpha Skin', 'Extra Widget'],
        },
      ],
    });
    const node = await hangarWidget.render(ownerCtx(), 'expanded');
    const { container } = render(node as React.ReactElement);

    // One row per contained item — not one opaque bundle row.
    const rows = container.querySelectorAll('.hud-readout-row');
    expect(rows.length).toBe(3);
    expect(container.textContent).toContain('Aegis Avenger Titan');
    expect(container.textContent).toContain('Alpha Skin');
    expect(container.textContent).toContain('Extra Widget');
    // The bundle name ties the items to their parent via the value column.
    expect(container.textContent).toContain('Gear – HighSec');
  });

  it('deep-links a resolvable contained ship to the KB (anchors where they resolve)', async () => {
    const railen: ReferenceEntry = {
      category: 'vehicle',
      class_name: 'Aegis Avenger Titan',
      display_name: 'Aegis Avenger Titan',
      slug: 'aegis-avenger-titan',
      summary: { category: 'vehicle' },
    };
    (loadAllReferenceBundles as ReturnType<typeof vi.fn>).mockResolvedValueOnce({
      catalogs: {
        vehicles: new Map([['aegis avenger titan', railen]]),
        weapons: new Map(),
        items: new Map(),
        locations: new Map(),
      },
    });
    mockHangar().mockResolvedValue({
      captured_at: '2026-07-22T00:00:00Z',
      ships: [
        {
          name: 'Gear - HighSec - Bundle',
          kind: 'Gear',
          contains: ['Aegis Avenger Titan', 'Alpha Skin'],
        },
      ],
    });
    const node = await hangarWidget.render(ownerCtx(), 'expanded');
    const { container } = render(node as React.ReactElement);
    // The ship-like constituent deep-links; the non-resolving one stays plain.
    expect(
      container.querySelector('a[href="/kb/vehicle/aegis-avenger-titan"]'),
    ).not.toBeNull();
    expect(container.textContent).toContain('Alpha Skin');
  });

  it('leaves a normal ship unchanged (no contains → single row)', async () => {
    mockHangar().mockResolvedValue({
      captured_at: '2026-07-22T00:00:00Z',
      ships: [{ name: 'Standalone Ships - Railen', kind: 'ship' }],
    });
    const node = await hangarWidget.render(ownerCtx(), 'expanded');
    const { container } = render(node as React.ReactElement);
    const rows = container.querySelectorAll('.hud-readout-row');
    expect(rows.length).toBe(1);
    expect(container.textContent).toContain('Railen');
  });

  it('does NOT expand a single-element contains that just echoes the pledge', async () => {
    mockHangar().mockResolvedValue({
      captured_at: '2026-07-22T00:00:00Z',
      ships: [
        { name: 'Aegis Avenger Titan', kind: 'ship', contains: ['Aegis Avenger Titan'] },
      ],
    });
    const node = await hangarWidget.render(ownerCtx(), 'expanded');
    const { container } = render(node as React.ReactElement);
    const rows = container.querySelectorAll('.hud-readout-row');
    expect(rows.length).toBe(1);
    expect(container.textContent).toContain('Aegis Avenger Titan');
  });

  it('honours EXPANDED_CAP across the flattened bundle list', async () => {
    mockHangar().mockResolvedValue({
      captured_at: '2026-07-22T00:00:00Z',
      ships: [
        {
          name: 'Mega - Pack - Bundle',
          kind: 'Gear',
          contains: Array.from({ length: 15 }, (_, i) => `Item ${i}`),
        },
      ],
    });
    const node = await hangarWidget.render(ownerCtx(), 'expanded');
    const { container } = render(node as React.ReactElement);
    const rows = container.querySelectorAll('.hud-readout-row');
    expect(rows.length).toBe(12);
    expect(container.textContent).toContain('+3 more');
  });
});
