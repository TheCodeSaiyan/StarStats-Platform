import React from 'react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render } from '@testing-library/react';

// F9: a VISITOR's lifetime totals must come from the handle-scoped
// playtime aggregate (getUserPlaytime), gated by the same
// share_event_timeline grant as the session list — NOT undercounted from
// the 50-capped session list. The owner keeps the me-scoped all-time
// aggregate (getPlaytime). Neither path fetches the other's endpoint.
vi.mock('@/lib/api', () => ({
  getSessions: vi.fn(),
  getPlaytime: vi.fn(),
  getUserPlaytime: vi.fn(),
}));

import { getSessions, getPlaytime, getUserPlaytime } from '@/lib/api';
import { sessionsWidget } from './sessions';
import { DEFAULT_SHARE_SCOPES } from './types';
import type { ViewerCtx } from './types';

const asMock = (fn: unknown) => fn as ReturnType<typeof vi.fn>;

function ownerCtx(): ViewerCtx {
  return {
    ownerHandle: 'alice',
    viewerHandle: 'alice',
    isOwner: true,
    token: 'alice-tok',
    shareScopes: { ...DEFAULT_SHARE_SCOPES },
    recipientScopes: null,
    range: '30d',
  };
}

function visitorCtx(): ViewerCtx {
  return {
    ownerHandle: 'alice',
    viewerHandle: 'bob',
    isOwner: false,
    token: 'bob-tok',
    shareScopes: { ...DEFAULT_SHARE_SCOPES },
    recipientScopes: null,
    range: '30d',
  };
}

const SESSIONS = {
  sessions: [
    {
      id: 's1',
      started_at: '2026-01-01T00:00:00Z',
      ended_at: '2026-01-01T02:00:00Z',
      event_count: 10,
    },
  ],
};

const AGGREGATE = { total_playtime_secs: 360_000, session_count: 128 };

describe('sessionsWidget range-scoped aggregate (F9)', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    asMock(getSessions).mockResolvedValue(SESSIONS);
    asMock(getPlaytime).mockResolvedValue({ hours: 0, ...AGGREGATE });
    asMock(getUserPlaytime).mockResolvedValue(AGGREGATE);
  });

  it('owner render reads the me-scoped windowed aggregate (range → hours), not the handle-scoped one', async () => {
    const result = await sessionsWidget.render(ownerCtx(), 'compact');
    expect(result).not.toBeNull();
    // ctx.range '30d' → rangeToHours = 720; windowed (allTime=false).
    expect(getPlaytime).toHaveBeenCalledWith('alice-tok', 720, false);
    expect(getUserPlaytime).not.toHaveBeenCalled();
  });

  it('visitor render reads the handle-scoped windowed aggregate so totals are exact (F9), never the me-scoped one', async () => {
    const result = await sessionsWidget.render(visitorCtx(), 'compact');
    expect(result).not.toBeNull();
    expect(getUserPlaytime).toHaveBeenCalledWith('bob-tok', 'alice', 720);
    expect(getPlaytime).not.toHaveBeenCalled();
  });
});

describe('sessionsWidget last-5 playtime sparkline (M5)', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    asMock(getPlaytime).mockResolvedValue({ hours: 0, ...AGGREGATE });
    asMock(getUserPlaytime).mockResolvedValue(AGGREGATE);
  });

  it('renders an accessible sparkline when 2+ sessions have a duration', async () => {
    asMock(getSessions).mockResolvedValue({
      sessions: [
        { id: 's3', started_at: '2026-01-03T00:00:00Z', ended_at: '2026-01-03T01:00:00Z', event_count: 5 },
        { id: 's2', started_at: '2026-01-02T00:00:00Z', ended_at: '2026-01-02T00:30:00Z', event_count: 5 },
        { id: 's1', started_at: '2026-01-01T00:00:00Z', ended_at: '2026-01-01T02:00:00Z', event_count: 5 },
      ],
    });
    const el = await sessionsWidget.render(ownerCtx(), 'compact');
    expect(el).not.toBeNull();
    const { getByRole } = render(el!);
    // Label reflects the ACTUAL number of plotted sessions (3 here), not a
    // hardcoded "5" — an honest window size.
    const spark = getByRole('img', { name: 'session playtime, last 3 sessions' });
    expect(spark.tagName.toLowerCase()).toBe('svg');
  });

  it('omits the sparkline when only one session has a duration', async () => {
    asMock(getSessions).mockResolvedValue(SESSIONS); // single session
    const el = await sessionsWidget.render(ownerCtx(), 'compact');
    expect(el).not.toBeNull();
    const { queryByRole } = render(el!);
    expect(queryByRole('img', { name: 'session playtime, last 5 sessions' })).toBeNull();
  });
});
