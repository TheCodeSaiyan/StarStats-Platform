import React from 'react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render } from '@testing-library/react';

vi.mock('@/lib/api', () => ({
  getStabilityStats: vi.fn(),
  getPlaytime: vi.fn(),
}));

import { getStabilityStats, getPlaytime } from '@/lib/api';
import { stabilityWidget } from './stability';
import { DEFAULT_SHARE_SCOPES } from './types';
import type { ViewerCtx } from './types';

function ownerCtx(): ViewerCtx {
  return {
    token: 't',
    isOwner: true,
    ownerHandle: 'TestPilot',
    range: '7d',
    shareScopes: DEFAULT_SHARE_SCOPES,
  } as ViewerCtx;
}

type Data = {
  crashes: number;
  perHour: number | null;
  hoursPlayed: number;
} | null;

describe('stabilityWidget', () => {
  beforeEach(() => vi.clearAllMocks());

  it('reports a rate, because a raw crash count says nothing on its own', async () => {
    // Four crashes across forty hours is a very different result from four
    // across four, and the count alone cannot tell them apart.
    vi.mocked(getStabilityStats).mockResolvedValue({
      hours: 168,
      crashes: 4,
      by_channel: [{ value: 'LIVE', count: 4 }],
    } as never);
    vi.mocked(getPlaytime).mockResolvedValue({
      total_playtime_secs: 40 * 3600,
      session_count: 12,
    } as never);

    const d = (await stabilityWidget.load!(ownerCtx())) as Data;
    expect(d!.crashes).toBe(4);
    expect(d!.perHour).toBeCloseTo(0.1, 5);
  });

  it('treats a clean window as a RESULT, not as no data', async () => {
    // Zero crashes is the best thing this widget can say. Returning null
    // would hide the good news and leave a gap where an answer belongs.
    vi.mocked(getStabilityStats).mockResolvedValue({
      hours: 168,
      crashes: 0,
      by_channel: [],
    } as never);
    vi.mocked(getPlaytime).mockResolvedValue({
      total_playtime_secs: 9 * 3600,
      session_count: 3,
    } as never);

    const d = (await stabilityWidget.load!(ownerCtx())) as Data;
    expect(d).not.toBeNull();
    expect(d!.crashes).toBe(0);
    expect(d!.perHour).toBe(0);
  });

  it('renders nothing when there was no playing and no crashing', async () => {
    vi.mocked(getStabilityStats).mockResolvedValue({
      hours: 168,
      crashes: 0,
      by_channel: [],
    } as never);
    vi.mocked(getPlaytime).mockResolvedValue({
      total_playtime_secs: 0,
      session_count: 0,
    } as never);

    expect(await stabilityWidget.load!(ownerCtx())).toBeNull();
  });

  it('still reports the count when playtime is unavailable', async () => {
    // The rate degrades to null; the crashes do not disappear with it.
    vi.mocked(getStabilityStats).mockResolvedValue({
      hours: 168,
      crashes: 3,
      by_channel: [],
    } as never);
    vi.mocked(getPlaytime).mockRejectedValue(new Error('boom'));

    const d = (await stabilityWidget.load!(ownerCtx())) as Data;
    expect(d!.crashes).toBe(3);
    expect(d!.perHour).toBeNull();
  });

  it('never loads for a visitor', async () => {
    // Crash data is me-scoped with no friend endpoint, so rendering for a
    // visitor would put the VIEWER's crashes on someone else's profile.
    const visitor = { ...ownerCtx(), isOwner: false } as ViewerCtx;
    expect(await stabilityWidget.load!(visitor)).toBeNull();
    expect(getStabilityStats).not.toHaveBeenCalled();
  });
});

describe('stability channel breakdown', () => {
  beforeEach(() => vi.clearAllMocks());

  /** Expanded render with a given by_channel set. */
  async function renderExpanded(by_channel: { value: string; count: number }[]) {
    vi.mocked(getStabilityStats).mockResolvedValue({
      hours: 168,
      crashes: by_channel.reduce((a, b) => a + b.count, 0),
      by_channel,
    } as never);
    vi.mocked(getPlaytime).mockResolvedValue({
      total_playtime_secs: 40 * 3600,
      session_count: 12,
    } as never);
    const node = await stabilityWidget.render(ownerCtx(), 'expanded');
    return render(node as React.ReactElement).container;
  }

  it('does not render a breakdown of one', async () => {
    // A single-channel player — the overwhelmingly common case — got a
    // one-row list whose only value IS `crashes`, restated directly beneath
    // itself. The guard was `by_channel.length === 0`, so length 1 fell
    // through to the RankedList. A breakdown of one is not a breakdown.
    const c = await renderExpanded([{ value: 'LIVE', count: 4 }]);
    // `.hud-readout-row` is RankedList's row element (kit/archetypes.tsx:104).
    // An earlier draft of this test asserted on `.hud-row`, which does not
    // exist anywhere — so it passed against the broken code and proved
    // nothing. Assert the element that actually differs.
    expect(c.querySelectorAll('.hud-readout-row')).toHaveLength(0);
    expect(c.textContent).not.toContain('LIVE');
    // The headline figure is still there — this hides the duplicate, not the data.
    expect(c.textContent).toContain('4');
  });

  it('still renders the breakdown when there is something to break down', async () => {
    const c = await renderExpanded([
      { value: 'LIVE', count: 3 },
      { value: 'PTU', count: 1 },
    ]);
    expect(c.querySelectorAll('.hud-readout-row')).toHaveLength(2);
    expect(c.textContent).toContain('LIVE');
    expect(c.textContent).toContain('PTU');
  });
});
