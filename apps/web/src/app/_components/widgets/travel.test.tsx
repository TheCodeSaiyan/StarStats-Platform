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
      planets_visited: [
        { value: 'Crusader', count: 3 },
        { value: 'microTech', count: 2 },
      ],
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

  it('expanded shows top routes + a "See travel map" link (no raw event dump)', async () => {
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
      top_destinations: [{ value: 'Stanton_Crusader_Orison', count: 4 }],
    });

    const node = await travelWidget.render(ownerCtx('7d'), 'expanded');
    const { container } = render(node as React.ReactElement);

    expect(container.textContent).toContain('Top routes');
    expect(container.textContent).toContain('Crusader');
    // Full map depth lives behind a link, not an inline (clipping) panel.
    expect(container.textContent).toContain('See travel map');
    const link = container.querySelector('a[href="/me/travel"]');
    expect(link).not.toBeNull();
    // The raw event-type dump is gone.
    expect(container.textContent).not.toContain('Joined PU');
  });
});

describe('travelWidget location KB links (classKey = friendly label)', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('links a real location (catalog display_name match) and leaves a synthetic label as plain text', async () => {
    (getMetricsEventTypes as ReturnType<typeof vi.fn>).mockResolvedValue({
      types: [{ event_type: 'quantum_target_selected', count: 5 }],
    });
    (getRoutes as ReturnType<typeof vi.fn>).mockResolvedValue({
      routes: [
        // Real place: friendly label "microTech" hits the dual-keyed
        // catalog by display_name → resolves a slug → KB link.
        { destination: 'microTech', count: 4 },
        // Synthetic per-mission beacon → "Mission beacon" → no catalog
        // match → plain text.
        { destination: 'MISSION_QT_Quantum_Beacon_718', count: 2 },
      ],
    });
    // Populate the locations catalog keyed by lowercased display_name
    // (dual-keying), mirroring how loadAllReferenceBundles resolves.
    const microTech: ReferenceEntry = {
      category: 'location',
      class_name: 'microTech',
      display_name: 'microTech',
      slug: 'microtech',
      summary: { category: 'location' },
    };
    (loadAllReferenceBundles as ReturnType<typeof vi.fn>).mockResolvedValueOnce({
      catalogs: { locations: new Map([['microtech', microTech]]) },
    });

    const node = await travelWidget.render(ownerCtx('7d'), 'expanded');
    const { container } = render(node as React.ReactElement);

    // Real location resolves to a KB deep-link.
    const link = container.querySelector('a[href="/kb/location/microtech"]');
    expect(link).not.toBeNull();
    expect(link?.textContent).toBe('microTech');

    // The synthetic beacon label is present but NOT wrapped in an anchor.
    expect(container.textContent).toContain('Mission beacon');
    const anchorTexts = Array.from(container.querySelectorAll('a')).map(
      (a) => a.textContent?.trim() ?? '',
    );
    expect(anchorTexts).not.toContain('Mission beacon');
  });
});

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
