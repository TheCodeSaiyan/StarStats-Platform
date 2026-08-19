import React from 'react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render } from '@testing-library/react';

vi.mock('@/lib/api', () => ({
  getMetricsEventTypes: vi.fn(),
  getObjectives: vi.fn(),
}));

import { getMetricsEventTypes, getObjectives } from '@/lib/api';
import { combatMissionWidget } from './combat_mission';
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

describe('combatMissionWidget range-awareness', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('is marked range-aware', () => {
    expect(combatMissionWidget.rangeAware).toBe(true);
  });

  it('passes ctx.range to getMetricsEventTypes', async () => {
    (getMetricsEventTypes as ReturnType<typeof vi.fn>).mockResolvedValue({
      types: [{ event_type: 'player_death', count: 3 }],
    });

    await combatMissionWidget.render(ownerCtx('90d'), 'compact');

    expect(getMetricsEventTypes).toHaveBeenCalledWith('tok', '90d');
  });

  it('sends a real 24h window now the endpoint serves that bucket', async () => {
    // Previously asserted 24h -> 7d, which encoded a server limitation
    // as intent: the widget rendered a WEEK under a "24h" label. The
    // endpoint gained a 24h bucket, so widening would now be the bug.
    (getMetricsEventTypes as ReturnType<typeof vi.fn>).mockResolvedValue({
      types: [{ event_type: 'player_death', count: 1 }],
    });

    await combatMissionWidget.render(ownerCtx('24h'), 'compact');

    expect(getMetricsEventTypes).toHaveBeenCalledWith('tok', '24h');
  });

  it('renders non-null when snake_case event types match', async () => {
    (getMetricsEventTypes as ReturnType<typeof vi.fn>).mockResolvedValue({
      types: [
        { event_type: 'player_death', count: 3 },
        { event_type: 'player_incapacitated', count: 1 },
        { event_type: 'actor_death', count: 2 },
        { event_type: 'mission_start', count: 5 },
        { event_type: 'mission_end', count: 4 },
      ],
    });

    const result = await combatMissionWidget.render(ownerCtx('90d'), 'compact');

    expect(result).not.toBeNull();
  });

  it('surfaces objective completion % from getObjectives', async () => {
    (getMetricsEventTypes as ReturnType<typeof vi.fn>).mockResolvedValue({
      types: [{ event_type: 'mission_start', count: 4 }],
    });
    (getObjectives as ReturnType<typeof vi.fn>).mockResolvedValue({
      completed: 3,
      failed: 1,
      in_progress: 1,
      unresolved: 0,
      total: 5,
      completion_pct: 75,
    });

    const node = await combatMissionWidget.render(ownerCtx('90d'), 'compact');
    const { container } = render(node as React.ReactElement);

    // 90d => 24*90 = 2160 hours. Objectives MUST be range-scoped, not lifetime.
    expect(getObjectives).toHaveBeenCalledWith('tok', 2160);
    expect(container.textContent).toContain('75%');
    expect(container.textContent).toContain('obj done');
  });

  it('passes the ctx.range window (hours) to getObjectives', async () => {
    (getMetricsEventTypes as ReturnType<typeof vi.fn>).mockResolvedValue({
      types: [{ event_type: 'mission_start', count: 4 }],
    });
    (getObjectives as ReturnType<typeof vi.fn>).mockResolvedValue({
      completed: 3,
      failed: 1,
      in_progress: 0,
      unresolved: 0,
      total: 4,
      completion_pct: 75,
    });

    await combatMissionWidget.render(ownerCtx('30d'), 'compact');

    // 30d => 24*30 = 720 hours, passed as the 2nd arg.
    expect(getObjectives).toHaveBeenCalledWith('tok', 720);
  });

  it('never fetches objectives unscoped (lifetime) while metrics are range-scoped', async () => {
    // Regression guard: `getObjectives(token)` with no hours returned
    // all-time totals, so the tile rendered a 30-day combat breakdown
    // beside a lifetime objective % under one range label.
    (getMetricsEventTypes as ReturnType<typeof vi.fn>).mockResolvedValue({
      types: [{ event_type: 'mission_start', count: 4 }],
    });
    (getObjectives as ReturnType<typeof vi.fn>).mockResolvedValue({
      completed: 1,
      failed: 0,
      in_progress: 0,
      unresolved: 0,
      total: 1,
      completion_pct: 100,
    });

    await combatMissionWidget.render(ownerCtx('7d'), 'compact');

    const objectivesArgs = (getObjectives as ReturnType<typeof vi.fn>).mock.calls[0];
    expect(objectivesArgs).toHaveLength(2);
    expect(objectivesArgs[1]).toBe(168);
    expect(objectivesArgs[1]).not.toBeUndefined();
  });
});

describe('combatMissionWidget C2 owner-only gating', () => {
  const visitorCtx: ViewerCtx = {
    ownerHandle: 'alice',
    viewerHandle: 'bob',
    isOwner: false,
    token: 'bob-tok',
    shareScopes: { ...DEFAULT_SHARE_SCOPES, combat_mission: true },
    recipientScopes: null,
    range: '7d',
  };

  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('is available to the owner', () => {
    expect(combatMissionWidget.isAvailable(ownerCtx('7d'))).toBe(true);
  });

  it('is UNavailable to a visitor even with the combat_mission share scope on', () => {
    // No friend-scoped metrics endpoint exists, so the widget must not
    // render for a visitor — it would surface the viewer's own data.
    expect(combatMissionWidget.isAvailable(visitorCtx)).toBe(false);
  });

  it('render returns null for a visitor without calling the me endpoint', async () => {
    const result = await combatMissionWidget.render(visitorCtx, 'compact');
    expect(result).toBeNull();
    expect(getMetricsEventTypes).not.toHaveBeenCalled();
  });
});
