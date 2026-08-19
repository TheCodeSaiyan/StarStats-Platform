import React from 'react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render } from '@testing-library/react';

vi.mock('@/lib/api', () => ({
  getRoutes: vi.fn(),
}));
vi.mock('@/lib/reference', () => ({
  loadAllReferenceBundles: vi
    .fn()
    .mockResolvedValue({ catalogs: { locations: undefined } }),
}));

import { getRoutes } from '@/lib/api';
import { routesWidget } from './routes';
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

/** Two destinations, 5 + 4 = 9 trips in the window. */
function fixture() {
  return {
    routes: [
      { destination: 'microTech', count: 5 },
      { destination: 'Crusader', count: 4 },
    ],
  };
}

const mockRoutes = () => getRoutes as ReturnType<typeof vi.fn>;

describe('routesWidget', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('is range-aware and owner-only', async () => {
    expect(routesWidget.rangeAware ?? false).toBe(true);
    expect(await routesWidget.isAvailable(ownerCtx(true))).toBe(true);
    expect(await routesWidget.isAvailable(ownerCtx(false))).toBe(false);
  });

  it('renders ranked destinations with the quantum-travel caveat', async () => {
    mockRoutes().mockResolvedValue(fixture());
    const node = await routesWidget.render(ownerCtx(), 'compact');
    const { container } = render(node as React.ReactElement);
    expect(container.querySelectorAll('.hud-readout-row').length).toBe(2);
    expect(container.textContent).toContain('5');
    expect(container.textContent).toContain('Based on quantum travel');
  });

  it('compares the windowed trips against the lifetime baseline', async () => {
    mockRoutes().mockResolvedValue({
      ...fixture(),
      lifetime: { total_trips: 188, destinations: 22 },
    });
    const node = await routesWidget.render(ownerCtx(), 'compact');
    const { container } = render(node as React.ReactElement);
    // 5 + 4 = 9 trips this window, against the lifetime trip count.
    expect(container.textContent).toContain('9 of 188 trips all time');
    // The quantum-travel caveat is joined to the comparison, not replaced.
    expect(container.textContent).toContain('Based on quantum travel');
    // `destinations` is a RAW distinct-destination count while the rows are
    // merged buckets, so it is deliberately never rendered — the two are
    // different quantities and pairing them would misread as a comparison.
    expect(container.textContent).not.toContain('22');
  });

  // `all` is a real 8760h window, so the server DOES send a twin for it
  // — but that twin spans the same rows, so the note would compare the
  // list to itself. Suppressed client-side; the caveat stays.
  it('renders no comparison on the "all" range even when the API sends a twin', async () => {
    mockRoutes().mockResolvedValue({
      routes: [{ destination: 'microTech', count: 188 }],
      lifetime: { total_trips: 188, destinations: 1 },
    });
    const node = await routesWidget.render(ownerCtx(true, 'all'), 'compact');
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
    mockRoutes().mockResolvedValue({ ...fixture(), ...extra });
    const node = await routesWidget.render(ownerCtx(), 'compact');
    const { container } = render(node as React.ReactElement);
    // Own numbers and caveat still render …
    expect(container.textContent).toContain('5');
    expect(container.textContent).toContain('Based on quantum travel');
    // … but nothing that reads as a comparison against a baseline.
    expect(container.textContent).not.toContain('all time');
    expect(container.textContent).not.toMatch(/\d+ of \d+/);
  });

  it('returns null when there are no routes', async () => {
    mockRoutes().mockResolvedValue({ routes: [] });
    expect(await routesWidget.render(ownerCtx(), 'compact')).toBeNull();
  });

  it('returns null when the fetch rejects', async () => {
    mockRoutes().mockRejectedValue(new Error('boom'));
    expect(await routesWidget.render(ownerCtx(), 'compact')).toBeNull();
  });

  // Without `previous` seeded the widget takes the `?? lifetime`
  // fallback, leaving the trend branch unreachable under test.
  it('leads with the trend and drops the lifetime line', async () => {
    mockRoutes().mockResolvedValue({
      ...fixture(),
      lifetime: { total_trips: 188, destinations: 22 },
      previous: { total_trips: 6, destinations: 2 },
    });
    const node = await routesWidget.render(ownerCtx(), 'compact');
    const { container } = render(node as React.ReactElement);
    // 9 trips this window vs 6 before.
    expect(container.textContent).toContain('+3');
    expect(container.textContent).toContain('vs prev 30d');
    expect(container.textContent).not.toContain('all time');
    expect(container.textContent).toContain('Based on quantum travel');
  });

  // `destinations` is a distinct count over MERGED buckets, so a period
  // delta on it compares two different quantities. Trips only.
  it('does not trend the distinct destination count', async () => {
    mockRoutes().mockResolvedValue({
      ...fixture(),
      previous: { total_trips: 6, destinations: 22 },
    });
    const node = await routesWidget.render(ownerCtx(), 'compact');
    const { container } = render(node as React.ReactElement);
    const text = container.textContent ?? '';
    expect(text.match(/vs prev/g)).toHaveLength(1);
    expect(text).not.toMatch(/destination.*vs prev/);
  });

  // The bug as reported: "routes are not actually populating". The data
  // was there; #309 range-scoped these widgets on 2026-07-23, four days
  // after routes shipped, and a handle whose quantum travel predates
  // the selected range has been getting a blank tile ever since —
  // indistinguishable from a broken feature.
  it('says the window is empty rather than going blank when lifetime has routes', async () => {
    mockRoutes().mockResolvedValue({
      routes: [],
      lifetime: { total_trips: 188, destinations: 22 },
    });
    const node = await routesWidget.render(ownerCtx(), 'compact');
    expect(node).not.toBeNull();
    const { container } = render(node as React.ReactElement);
    // Names BOTH figures and the fix, so the user can tell "no data in
    // this window" from "this is broken".
    expect(container.textContent).toContain('188');
    expect(container.textContent).toMatch(/30d/);
    expect(container.textContent).toMatch(/widen the range/i);
  });

  // The other side: a handle with genuinely no routes must NOT be told
  // to widen the range — there is nothing wider to find.
  it('renders nothing at all when there are no routes in any window', async () => {
    mockRoutes().mockResolvedValue({
      routes: [],
      lifetime: { total_trips: 0, destinations: 0 },
    });
    const node = await routesWidget.render(ownerCtx(), 'compact');
    expect(node).toBeNull();
  });

  // `all` spans retention, so `lifetime` is deliberately null there. An
  // empty list on that range really does mean "no routes", and must not
  // suggest widening to a range that does not exist.
  it('renders nothing on the "all" range when the list is empty', async () => {
    mockRoutes().mockResolvedValue({ routes: [], lifetime: null });
    const node = await routesWidget.render(ownerCtx(true, 'all'), 'compact');
    expect(node).toBeNull();
  });
});
