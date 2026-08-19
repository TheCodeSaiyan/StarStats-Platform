import React from 'react';
import { describe, it, expect, vi, beforeEach } from 'vitest';

vi.mock('@/lib/api', () => ({
  getTimeline: vi.fn(),
  getFriendTimeline: vi.fn(),
}));
vi.mock('@/components/DayHeatmap', () => ({
  DayHeatmap: () => null,
}));

import { getTimeline } from '@/lib/api';
import { heatmapWidget } from './heatmap';
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

describe('heatmapWidget range-awareness', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('is marked range-aware', () => {
    expect(heatmapWidget.rangeAware).toBe(true);
  });

  it('requests days derived from ctx.range (not size)', async () => {
    (getTimeline as ReturnType<typeof vi.fn>).mockResolvedValue({
      buckets: [{ day: '2026-06-01', count: 1 }],
    });

    await heatmapWidget.render(ownerCtx('7d'), 'expanded');

    // 7d => 7 days, regardless of size.
    expect(getTimeline).toHaveBeenCalledWith('tok', { days: 7 });
  });
});
