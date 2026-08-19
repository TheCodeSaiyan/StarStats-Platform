import React from 'react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render } from '@testing-library/react';

vi.mock('@/lib/api', () => ({
  getLocationTrace: vi.fn(),
  getLocationBreakdown: vi.fn(),
}));

// Empty locations catalog → the chain-strip / timeline EntityLinks
// degrade to plain text (no real reference fetch, no next/link in jsdom).
vi.mock('@/lib/reference', () => ({
  loadAllReferenceBundles: vi
    .fn()
    .mockResolvedValue({ catalogs: { locations: new Map() } }),
}));

import { getLocationTrace, getLocationBreakdown } from '@/lib/api';
import { journeyWidget } from './journey';
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

const visitorCtx: ViewerCtx = {
  ownerHandle: 'alice',
  viewerHandle: 'bob',
  isOwner: false,
  token: 'bob-tok',
  shareScopes: { ...DEFAULT_SHARE_SCOPES },
  recipientScopes: null,
  range: '7d',
};

function trace(entries: unknown[]) {
  return { entries, hours: 168 };
}

const twoStops = [
  {
    started_at: '2026-07-01T10:00:00Z',
    ended_at: '2026-07-01T10:30:00Z',
    event_count: 3,
    system: 'Stanton',
    planet: 'Crusader',
    city: 'Orison',
    source_event_type: 'planet_terrain_load',
  },
  {
    started_at: '2026-07-02T12:00:00Z',
    ended_at: '2026-07-02T12:30:00Z',
    event_count: 5,
    system: 'Stanton',
    planet: 'microTech',
    city: 'New Babbage',
    source_event_type: 'planet_terrain_load',
  },
];

describe('journeyWidget wiring', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('is range-aware', () => {
    expect(journeyWidget.rangeAware).toBe(true);
  });

  it('is owner-only', () => {
    expect(journeyWidget.isAvailable(ownerCtx('7d'))).toBe(true);
    expect(journeyWidget.isAvailable(visitorCtx)).toBe(false);
  });

  it('returns null for a visitor without hitting the me endpoint', async () => {
    const result = await journeyWidget.render(visitorCtx, 'compact');
    expect(result).toBeNull();
    expect(getLocationTrace).not.toHaveBeenCalled();
  });

  it('passes the range window (hours) to getLocationTrace', async () => {
    (getLocationTrace as ReturnType<typeof vi.fn>).mockResolvedValue(trace([]));
    await journeyWidget.render(ownerCtx('7d'), 'compact');
    expect(getLocationTrace).toHaveBeenCalledWith('tok', 24 * 7);
  });
});

describe('journeyWidget empty states', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('shows the no-telemetry signal when the trace is empty (compact)', async () => {
    (getLocationTrace as ReturnType<typeof vi.fn>).mockResolvedValue(trace([]));
    const node = await journeyWidget.render(ownerCtx('7d'), 'compact');
    const { container } = render(node as React.ReactElement);
    expect(container.textContent).toContain('No Telemetry Signal Found');
  });

  it('shows the no-telemetry signal when the trace is empty (expanded)', async () => {
    (getLocationTrace as ReturnType<typeof vi.fn>).mockResolvedValue(trace([]));
    (getLocationBreakdown as ReturnType<typeof vi.fn>).mockResolvedValue({
      entries: [],
      hours: 168,
    });
    const node = await journeyWidget.render(ownerCtx('7d'), 'expanded');
    const { container } = render(node as React.ReactElement);
    expect(container.textContent).toContain('No Telemetry Signal Found');
  });
});

describe('journeyWidget rendering', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('renders the recent-stop chain in the compact size', async () => {
    (getLocationTrace as ReturnType<typeof vi.fn>).mockResolvedValue(trace(twoStops));
    const node = await journeyWidget.render(ownerCtx('7d'), 'compact');
    const { container } = render(node as React.ReactElement);
    expect(container.textContent).toContain('New Babbage');
  });

  it('renders the route map + timeline in the expanded size', async () => {
    (getLocationTrace as ReturnType<typeof vi.fn>).mockResolvedValue(trace(twoStops));
    (getLocationBreakdown as ReturnType<typeof vi.fn>).mockResolvedValue({
      entries: [],
      hours: 168,
    });
    const node = await journeyWidget.render(ownerCtx('7d'), 'expanded');
    const { container } = render(node as React.ReactElement);
    expect(container.textContent).toContain('Route map');
    expect(container.textContent).toContain('Stops');
    // Both distinct stops surface in the timeline.
    expect(container.textContent).toContain('Orison');
    expect(container.textContent).toContain('New Babbage');
  });
});
