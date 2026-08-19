import React from 'react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render } from '@testing-library/react';

vi.mock('@/lib/api', () => ({
  getFleet: vi.fn(),
}));
vi.mock('@/lib/reference', () => ({
  loadAllReferenceBundles: vi.fn().mockResolvedValue({ catalogs: { vehicles: undefined } }),
}));

import { getFleet } from '@/lib/api';
import { fleetWidget } from './fleet';
import { DEFAULT_SHARE_SCOPES, type ViewerCtx } from './types';

function ownerCtx(
  isOwner = true,
  range: ViewerCtx['range'] = '30d',
): ViewerCtx {
  return {
    ownerHandle: 'alice',
    viewerHandle: isOwner ? 'alice' : 'bob',
    isOwner,
    token: 'tok',
    shareScopes: { ...DEFAULT_SHARE_SCOPES },
    recipientScopes: null,
    range,
  };
}

const mockFleet = () => getFleet as ReturnType<typeof vi.fn>;

describe('fleetWidget', () => {
  beforeEach(() => { vi.clearAllMocks(); });

  it('is range-aware and owner-only', async () => {
    expect(fleetWidget.rangeAware ?? false).toBe(true);
    expect(await fleetWidget.isAvailable(ownerCtx(true))).toBe(true);
    expect(await fleetWidget.isAvailable(ownerCtx(false))).toBe(false);
  });

  it('renders ranked ships with the quantum-travel caveat', async () => {
    mockFleet().mockResolvedValue({
      ships: [
        { vehicle_class: 'AEGS_Gladius', trip_count: 42 },
        { vehicle_class: 'DRAK_Cutlass_Black', trip_count: 17 },
      ],
    });
    const node = await fleetWidget.render(ownerCtx(), 'compact');
    const { container } = render(node as React.ReactElement);
    expect(container.textContent).toContain('42');
    expect(container.textContent).toContain('Based on quantum travel');
  });

  it('caps the tile to the top ships and links the rest via "See all" (never scrolls)', async () => {
    // 9 ships → only the top 6 render; the remainder is a see-more link.
    mockFleet().mockResolvedValue({
      ships: Array.from({ length: 9 }, (_, i) => ({
        vehicle_class: `SHIP_${i}`,
        trip_count: 100 - i,
      })),
    });
    const node = await fleetWidget.render(ownerCtx(), 'compact');
    const { container } = render(node as React.ReactElement);
    const rows = container.querySelectorAll('.hud-readout-row');
    expect(rows.length).toBe(6);
    expect(container.textContent).toContain('See all 9');
    const link = container.querySelector('a');
    expect(link?.getAttribute('href')).toBe('/u/alice/entities');
  });

  it('shows no "See all" link when every ship already fits', async () => {
    mockFleet().mockResolvedValue({
      ships: [{ vehicle_class: 'AEGS_Gladius', trip_count: 42 }],
    });
    const node = await fleetWidget.render(ownerCtx(), 'compact');
    const { container } = render(node as React.ReactElement);
    expect(container.textContent).not.toContain('See all');
  });

  it('compares the windowed trips and ships against the lifetime baseline', async () => {
    mockFleet().mockResolvedValue({
      ships: [
        { vehicle_class: 'AEGS_Gladius', trip_count: 42 },
        { vehicle_class: 'DRAK_Cutlass_Black', trip_count: 17 },
      ],
      lifetime: { total_trips: 214, ships_flown: 11 },
    });
    const node = await fleetWidget.render(ownerCtx(), 'compact');
    const { container } = render(node as React.ReactElement);
    // 42 + 17 = 59 trips across 2 ships, each against its lifetime twin.
    expect(container.textContent).toContain('59 of 214 trips, 2 of 11 ships all time');
    // The quantum-travel caveat is joined to the comparison, not replaced.
    expect(container.textContent).toContain('Based on quantum travel');
  });

  // `all` is a real 8760h window, so the server DOES send a twin for it
  // — but that twin spans the same rows, so the note would compare the
  // list to itself. Suppressed client-side; the caveat stays.
  it('renders no comparison on the "all" range even when the API sends a twin', async () => {
    mockFleet().mockResolvedValue({
      ships: [{ vehicle_class: 'RSI_Aurora', trip_count: 214 }],
      lifetime: { total_trips: 214, ships_flown: 1 },
    });
    const node = await fleetWidget.render(ownerCtx(true, 'all'), 'compact');
    const { container } = render(node as React.ReactElement);
    expect(container.textContent).not.toMatch(/all time/i);
    expect(container.textContent).toContain('Based on quantum travel');
  });

  // `lifetime` is omitted whenever no window was requested (the 'all'
  // range). No baseline may be invented in its place.
  it.each([
    ['absent', {}],
    ['null', { lifetime: null }],
  ])('renders no comparison when the lifetime baseline is %s', async (_label, extra) => {
    mockFleet().mockResolvedValue({
      ships: [{ vehicle_class: 'AEGS_Gladius', trip_count: 42 }],
      ...extra,
    });
    const node = await fleetWidget.render(ownerCtx(), 'compact');
    const { container } = render(node as React.ReactElement);
    // Own numbers and caveat still render …
    expect(container.textContent).toContain('42');
    expect(container.textContent).toContain('Based on quantum travel');
    // … but nothing that reads as a comparison against a baseline.
    expect(container.textContent).not.toContain('all time');
    expect(container.textContent).not.toMatch(/\d+ of \d+/);
  });

  it('returns null when there are no ships', async () => {
    mockFleet().mockResolvedValue({ ships: [] });
    expect(await fleetWidget.render(ownerCtx(), 'compact')).toBeNull();
  });

  it('returns null when the fetch rejects', async () => {
    mockFleet().mockRejectedValue(new Error('boom'));
    expect(await fleetWidget.render(ownerCtx(), 'compact')).toBeNull();
  });

  // See the docking suite: without `previous` seeded, the trend branch
  // is unreachable and the lifetime fallback masks any error in it.
  it('leads with the period-over-period trend when a predecessor exists', async () => {
    mockFleet().mockResolvedValue({
      ships: [
        { vehicle_class: 'AEGS_Gladius', trip_count: 42 },
        { vehicle_class: 'DRAK_Cutlass_Black', trip_count: 17 },
      ],
      lifetime: { total_trips: 214, ships_flown: 11 },
      previous: { total_trips: 40, ships_flown: 2 },
    });
    const node = await fleetWidget.render(ownerCtx(), 'compact');
    const { container } = render(node as React.ReactElement);
    // 59 trips this window vs 40 before.
    expect(container.textContent).toContain('+19');
    expect(container.textContent).toContain('vs prev 30d');
    // Trend replaces the lifetime line.
    expect(container.textContent).not.toContain('all time');
    expect(container.textContent).toContain('Based on quantum travel');
  });

  // ships_flown is a DISTINCT count, so its period delta is not
  // additive — "+2 ships" would read as two new ships when it may be
  // the same pilots in a different mix. Trips only.
  it('does not trend the distinct ship count', async () => {
    mockFleet().mockResolvedValue({
      ships: [{ vehicle_class: 'AEGS_Gladius', trip_count: 42 }],
      previous: { total_trips: 40, ships_flown: 2 },
    });
    const node = await fleetWidget.render(ownerCtx(), 'compact');
    const { container } = render(node as React.ReactElement);
    // 42 vs 40 trips. The percentage IS shown here — 40 is above the
    // floor, unlike the docking case — so assert the parts rather than
    // a contiguous string.
    expect(container.textContent).toContain('+2');
    expect(container.textContent).toContain('vs prev 30d');
    // The ship count (1 this window vs 2 before) must NOT be trended.
    expect(container.textContent).not.toMatch(/ships.*vs prev/);
    expect(container.textContent).not.toContain('−1');
  });
});

// #363 gave these tiles an empty-window state; nothing pinned that it
// actually renders. See kit/EmptyWindow for the bug it fixes.
describe('fleetWidget empty window vs empty account', () => {
  beforeEach(() => { vi.clearAllMocks(); });

  it('says the window is empty rather than going blank when lifetime has trips', async () => {
    mockFleet().mockResolvedValue({ ships: [], lifetime: { total_trips: 188 } });
    const node = await fleetWidget.render(ownerCtx(), 'compact');
    expect(node).not.toBeNull();
    const { container } = render(node as React.ReactElement);
    expect(container.textContent).toContain('188');
    expect(container.textContent).toMatch(/30d/);
    expect(container.textContent).toMatch(/widen the range/i);
  });

  it('renders nothing at all when there are no trips in any window', async () => {
    mockFleet().mockResolvedValue({ ships: [], lifetime: { total_trips: 0 } });
    expect(await fleetWidget.render(ownerCtx(), 'compact')).toBeNull();
  });

  it('renders nothing on the "all" range when the window is empty', async () => {
    mockFleet().mockResolvedValue({ ships: [], lifetime: { total_trips: 188 } });
    expect(await fleetWidget.render(ownerCtx(true, 'all'), 'compact')).toBeNull();
  });
});
