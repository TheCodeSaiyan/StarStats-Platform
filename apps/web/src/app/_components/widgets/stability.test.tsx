import { describe, it, expect, vi, beforeEach } from 'vitest';

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
