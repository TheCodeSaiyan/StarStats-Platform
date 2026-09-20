import React from 'react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render } from '@testing-library/react';

vi.mock('@/lib/api', () => ({
  getMetricsEventTypes: vi.fn(),
  getRoutes: vi.fn(),
  getTravelStats: vi.fn(),
  getLocationsVisited: vi.fn(),
}));

// Keep the rest of @/lib/reference real (HierarchicalBucketList needs
// prettyClass + types); only stub the catalog fetch so no network call
// fires from the widget's Promise.allSettled.
vi.mock('@/lib/reference', async (importActual) => {
  const actual = await importActual<typeof import('@/lib/reference')>();
  return {
    ...actual,
    getLocationCatalog: vi.fn().mockResolvedValue(actual.EMPTY_LOCATION_CATALOG),
    // Empty locations catalog → EntityLink degrades to plain text (no
    // real KB fetch, no next/link in jsdom), keeping the test hermetic.
    loadAllReferenceBundles: vi
      .fn()
      .mockResolvedValue({ catalogs: { locations: new Map() } }),
  };
});

import { getMetricsEventTypes, getRoutes, getTravelStats } from '@/lib/api';
import { loadAllReferenceBundles } from '@/lib/reference';
import type { ReferenceEntry } from '@/lib/reference-types';
import { travelWidget } from './travel';
import { DEFAULT_SHARE_SCOPES } from './types';
import type { ViewerCtx } from './types';

function ownerCtx(range: ViewerCtx['range']): ViewerCtx {
  return {
    ownerHandle: 'alice',
    viewerHandle: 'alice',
    isOwner: true,
    token: 'tok',
    shareScopes: { ...DEFAULT_SHARE_SCOPES },
    recipientScopes: null,
    range,
  };
}

describe('travelWidget range-awareness', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('is marked range-aware', () => {
    expect(travelWidget.rangeAware).toBe(true);
  });

  it('passes ctx.range to getMetricsEventTypes', async () => {
    (getMetricsEventTypes as ReturnType<typeof vi.fn>).mockResolvedValue({
      types: [{ event_type: 'quantum_target_selected', count: 5 }],
    });

    await travelWidget.render(ownerCtx('7d'), 'compact');

    expect(getMetricsEventTypes).toHaveBeenCalledWith('tok', '7d');
  });

  it('renders non-null when snake_case event types match', async () => {
    (getMetricsEventTypes as ReturnType<typeof vi.fn>).mockResolvedValue({
      types: [
        { event_type: 'quantum_target_selected', count: 5 },
        { event_type: 'join_pu', count: 2 },
        { event_type: 'change_server', count: 1 },
      ],
    });

    const result = await travelWidget.render(ownerCtx('7d'), 'compact');

    expect(result).not.toBeNull();
  });

  it('surfaces the top quantum route from getRoutes', async () => {
    (getMetricsEventTypes as ReturnType<typeof vi.fn>).mockResolvedValue({
      types: [{ event_type: 'quantum_target_selected', count: 5 }],
    });
    (getRoutes as ReturnType<typeof vi.fn>).mockResolvedValue({
      routes: [
        { destination: 'Crusader', count: 4 },
        { destination: 'microTech', count: 1 },
      ],
    });

    const node = await travelWidget.render(ownerCtx('7d'), 'compact');
    const { container } = render(node as React.ReactElement);

    // 7d => 168 hours. Routes MUST share the travel-stats window.
    expect(getRoutes).toHaveBeenCalledWith('tok', 168);
    expect(container.textContent).toContain('Top route:');
    expect(container.textContent).toContain('Crusader');
  });

  it('passes the ctx.range window (hours) to getRoutes', async () => {
    (getMetricsEventTypes as ReturnType<typeof vi.fn>).mockResolvedValue({
      types: [{ event_type: 'quantum_target_selected', count: 5 }],
    });
    (getRoutes as ReturnType<typeof vi.fn>).mockResolvedValue({
      routes: [{ destination: 'Crusader', count: 4 }],
    });

    await travelWidget.render(ownerCtx('30d'), 'compact');

    // 30d => 24*30 = 720 hours, passed as the 2nd arg.
    expect(getRoutes).toHaveBeenCalledWith('tok', 720);
  });

  it('scopes routes to the SAME window as getTravelStats', async () => {
    // Regression guard: `getRoutes(token)` with no hours returned
    // lifetime top routes rendered beside range-scoped quantum/hop
    // counts under one range label.
    (getMetricsEventTypes as ReturnType<typeof vi.fn>).mockResolvedValue({
      types: [{ event_type: 'quantum_target_selected', count: 5 }],
    });
    (getRoutes as ReturnType<typeof vi.fn>).mockResolvedValue({
      routes: [{ destination: 'Crusader', count: 4 }],
    });
    (getTravelStats as ReturnType<typeof vi.fn>).mockResolvedValue({
      hours: 2160,
      quantum_jumps: 5,
      planets_visited: [],
      top_destinations: [],
    });

    await travelWidget.render(ownerCtx('90d'), 'compact');

    const travelHours = (getTravelStats as ReturnType<typeof vi.fn>).mock.calls[0][1];
    const routesArgs = (getRoutes as ReturnType<typeof vi.fn>).mock.calls[0];
    expect(routesArgs).toHaveLength(2);
    expect(routesArgs[1]).toBe(travelHours);
    expect(routesArgs[1]).toBe(2160);
  });
});

describe('travelWidget metric depth (getTravelStats)', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('prefers the server quantum_jumps aggregate and shows planets visited', async () => {
    (getMetricsEventTypes as ReturnType<typeof vi.fn>).mockResolvedValue({
      types: [{ event_type: 'quantum_target_selected', count: 5 }],
    });
    (getTravelStats as ReturnType<typeof vi.fn>).mockResolvedValue({
      hours: 168,
      quantum_jumps: 42,
      // A real count now, not a bucket list whose length was the figure.
      // The server caps nothing: 150 distinct planets reports 150, where the
      // old breakdown pinned at STATS_BUCKET_LIMIT (100).
      distinct_planets: 2,
      top_destinations: [{ value: 'Stanton_Crusader_Orison', count: 4 }],
    });

    const node = await travelWidget.render(ownerCtx('7d'), 'compact');
    const { container } = render(node as React.ReactElement);

    // Real aggregate (42) wins over the raw target-selection count (5).
    expect(container.textContent).toContain('42');
    // Planets-visited readout appears from planets_visited.length.
    expect(container.textContent).toContain('planets');
    expect(container.textContent).toContain('2');
  });
});

// The KB-link tests that stood here moved to routes.test.tsx with the list
// they exercised: `travel` no longer renders one, and the EntityLink behaviour
// they covered is `routes`' now. Relocated, not dropped.

describe('travelWidget C2 owner-only gating', () => {
  const visitorCtx: ViewerCtx = {
    ownerHandle: 'alice',
    viewerHandle: 'bob',
    isOwner: false,
    token: 'bob-tok',
    shareScopes: { ...DEFAULT_SHARE_SCOPES, travel: true },
    recipientScopes: null,
    range: '7d',
  };

  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('is available to the owner', () => {
    expect(travelWidget.isAvailable(ownerCtx('7d'))).toBe(true);
  });

  it('is UNavailable to a visitor even with the travel share scope on', () => {
    // No friend-scoped metrics endpoint exists, so the widget must not
    // render for a visitor — it would surface the viewer's own data.
    expect(travelWidget.isAvailable(visitorCtx)).toBe(false);
  });

  it('render returns null for a visitor without calling the me endpoint', async () => {
    const result = await travelWidget.render(visitorCtx, 'compact');
    expect(result).toBeNull();
    expect(getMetricsEventTypes).not.toHaveBeenCalled();
  });
});

describe('travel does not restate the routes widget', () => {
  beforeEach(() => vi.clearAllMocks());

  // Expanded used to render the same aggregated route list as the `routes`
  // widget, off the same getRoutes call, in the same lens. What survives here
  // is what only this tile reports — quantum, server hops, planets — plus the
  // top route as a NOTE and a link out to the map. The ranked list, and the
  // EntityLink coverage that went with it, moved to routes.test.tsx.
  it('shows the summary figures and the map link, not a route list', async () => {
    (getMetricsEventTypes as ReturnType<typeof vi.fn>).mockResolvedValue({
      types: [{ event_type: 'quantum_target_selected', count: 5 }],
    });
    (getRoutes as ReturnType<typeof vi.fn>).mockResolvedValue({
      routes: [
        { destination: 'Crusader', count: 4 },
        { destination: 'microTech', count: 1 },
      ],
    });
    (getTravelStats as ReturnType<typeof vi.fn>).mockResolvedValue({
      hours: 168,
      quantum_jumps: 5,
      planets_visited: [],
      top_destinations: [],
    });

    const node = await travelWidget.render(ownerCtx('7d'), 'expanded');
    const { container } = render(node as React.ReactElement);

    // No ranked rows — that list belongs to `routes`.
    expect(container.querySelectorAll('.hud-readout-row')).toHaveLength(0);
    expect(container.textContent).not.toContain('Top routes');

    // The top route still gets a mention, as a note rather than a list.
    expect(container.textContent).toContain('Crusader');
    // Depth lives behind the link, not an inline panel.
    expect(container.querySelector('a[href="/me/travel"]')).not.toBeNull();
    // And the raw event-type dump stays gone.
    expect(container.textContent).not.toContain('Joined PU');
  });
});
