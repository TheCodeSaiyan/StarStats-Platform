import React from 'react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render } from '@testing-library/react';

vi.mock('@/lib/api', () => ({
  getLocationTrace: vi.fn(),
}));

import { getLocationTrace } from '@/lib/api';
import { corridorsWidget } from './corridors';
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

// Two distinct stops in sequence → one undirected corridor (Orison ⇄ New
// Babbage) with a trip count of 1.
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

describe('corridorsWidget wiring', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('is range-aware', () => {
    expect(corridorsWidget.rangeAware).toBe(true);
  });

  it('is owner-only', () => {
    expect(corridorsWidget.isAvailable(ownerCtx('7d'))).toBe(true);
    expect(corridorsWidget.isAvailable(visitorCtx)).toBe(false);
  });

  it('returns null for a visitor without hitting the me endpoint', async () => {
    const result = await corridorsWidget.render(visitorCtx, 'expanded');
    expect(result).toBeNull();
    expect(getLocationTrace).not.toHaveBeenCalled();
  });

  it('passes the range window (hours) to getLocationTrace', async () => {
    (getLocationTrace as ReturnType<typeof vi.fn>).mockResolvedValue(trace([]));
    await corridorsWidget.render(ownerCtx('7d'), 'expanded');
    expect(getLocationTrace).toHaveBeenCalledWith('tok', 24 * 7);
  });
});

describe('corridorsWidget empty states', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  // Previously this returned null, and the canvas then rendered "No
  // activity recorded in this window" — untrue for a player who WAS
  // active but stayed in one place, and the reason the widget read as
  // broken. Three states used to collapse into that one message; the
  // widget now separates them.
  it('reports staying put, rather than claiming no activity', async () => {
    (getLocationTrace as ReturnType<typeof vi.fn>).mockResolvedValue(
      trace([twoStops[0]]),
    );
    const node = await corridorsWidget.render(ownerCtx('7d'), 'expanded');
    expect(node).not.toBeNull();
    const { container } = render(node as React.ReactElement);
    // Names WHERE they were: a fact, not an absence.
    expect(container.textContent).toContain('Orison');
    expect(container.textContent).toMatch(/no travel/i);
    // Must NOT claim there was no activity — there was.
    expect(container.textContent).not.toMatch(/no activity recorded/i);
  });

  // The one case where "no activity in this window" IS the truth: no
  // location telemetry at all. Still returns null so the canvas says it.
  it('renders nothing when the trace is empty', async () => {
    (getLocationTrace as ReturnType<typeof vi.fn>).mockResolvedValue(trace([]));
    const node = await corridorsWidget.render(ownerCtx('7d'), 'expanded');
    expect(node).toBeNull();
  });
});

describe('corridorsWidget rendering', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  // Reported as "the corridor widget is still not showing the actual
  // corridors, just a count". A tile called Corridors whose compact size
  // renders "1 corridors" answers a question nobody asked — the corridor
  // IS the datum, the count is metadata about it. Compact now leads with
  // the busiest leg and keeps the total in the note, so nothing is lost.
  it('names the busiest corridor in the compact size, not just a count', async () => {
    (getLocationTrace as ReturnType<typeof vi.fn>).mockResolvedValue(trace(twoStops));
    const node = await corridorsWidget.render(ownerCtx('7d'), 'compact');
    const { container } = render(node as React.ReactElement);
    expect(container.textContent).toContain('Orison');
    expect(container.textContent).toContain('New Babbage');
    expect(container.textContent).toContain('⇄');
  });

  it('still reports the total in the compact size', async () => {
    (getLocationTrace as ReturnType<typeof vi.fn>).mockResolvedValue(trace(twoStops));
    const node = await corridorsWidget.render(ownerCtx('7d'), 'compact');
    const { container } = render(node as React.ReactElement);
    // One corridor here, so it is the only one — the copy says so rather
    // than claiming "busiest of 1".
    expect(container.textContent).toMatch(/only corridor/i);
  });

  it('renders the top corridor rows in the expanded size', async () => {
    (getLocationTrace as ReturnType<typeof vi.fn>).mockResolvedValue(trace(twoStops));
    const node = await corridorsWidget.render(ownerCtx('7d'), 'expanded');
    const { container } = render(node as React.ReactElement);
    expect(container.textContent).toContain('Orison');
    expect(container.textContent).toContain('New Babbage');
    expect(container.textContent).toContain('⇄');
  });
});
