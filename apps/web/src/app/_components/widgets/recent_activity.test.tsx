import React from 'react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen } from '@testing-library/react';

vi.mock('@/lib/api', () => ({
  listEvents: vi.fn(),
  getFriendEvents: vi.fn(),
}));

import { getFriendEvents, listEvents } from '@/lib/api';
import {
  recentActivityWidget,
  isLowSignal,
  type RecentActivityData,
} from './recent_activity';
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

describe('recentActivityWidget range-awareness', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('is marked range-aware', () => {
    expect(recentActivityWidget.rangeAware).toBe(true);
  });

  it('passes a since ISO string derived from ctx.range to listEvents', async () => {
    (listEvents as ReturnType<typeof vi.fn>).mockResolvedValue({ events: [] });

    await recentActivityWidget.render(ownerCtx('7d'), 'compact');

    expect(listEvents).toHaveBeenCalledWith(
      'tok',
      expect.objectContaining({ since: expect.any(String) }),
    );
    const call = (listEvents as ReturnType<typeof vi.fn>).mock.calls[0] as [
      string,
      { since?: string },
    ];
    expect(call[1].since).toBeTruthy();
  });
});

describe('recentActivityWidget share-scope gating', () => {
  const visitorCtx: ViewerCtx = {
    ownerHandle: 'alice',
    viewerHandle: 'bob',
    isOwner: false,
    token: 'bob-tok',
    shareScopes: { ...DEFAULT_SHARE_SCOPES, recent_activity: true },
    recipientScopes: null,
    range: '7d',
  };

  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('is available to the owner', () => {
    expect(recentActivityWidget.isAvailable(ownerCtx('7d'))).toBe(true);
  });

  // THIS TEST REPLACES ONE ASSERTING THE OPPOSITE. The C2 rule was
  // "unavailable to a visitor even with the share scope on", and the
  // reason was written down: `/v1/me/events` had no friend-scoped
  // equivalent, so rendering for a visitor would have shown the VIEWER
  // their own events under the owner's name. `GET /v1/u/{handle}/events`
  // now exists, which removes the reason rather than overriding it. The
  // pilot's own switch — which named this widget all along — is the gate.
  it('is available to a visitor the pilot has turned the switch on for', () => {
    expect(recentActivityWidget.isAvailable(visitorCtx)).toBe(true);
  });

  it('stays hidden from a visitor when the switch is off', () => {
    // The default. Nothing becomes visible until the owner says so.
    expect(
      recentActivityWidget.isAvailable({
        ...visitorCtx,
        shareScopes: { ...DEFAULT_SHARE_SCOPES, recent_activity: false },
      }),
    ).toBe(false);
  });

  it('never reads the me-scoped endpoint on a visitor render', async () => {
    // The failure the old gate existed to prevent, pinned directly:
    // whatever the gate says, a visitor's render must not fetch
    // `/v1/me/events` with the visitor's token, because that returns the
    // VIEWER's events and would render them as the owner's.
    (getFriendEvents as ReturnType<typeof vi.fn>).mockResolvedValue({
      owner_handle: 'alice',
      events: [],
      next_before: null,
    });
    await recentActivityWidget.render(visitorCtx, 'compact');
    expect(listEvents).not.toHaveBeenCalled();
    expect(getFriendEvents).toHaveBeenCalledWith('bob-tok', 'alice', {
      limit: expect.any(Number),
    });
  });
});

describe('recentActivityWidget H9 label formatting', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('renders a humanised label, never the raw snake_case event_type', async () => {
    (listEvents as ReturnType<typeof vi.fn>).mockResolvedValue({
      events: [
        // A type the widget shows: `quantum_target_selected` (the original
        // fixture) is In-Transit noise and is now hidden with the rest.
        {
          seq: 1,
          event_type: 'vehicle_destruction',
          event_timestamp: '2026-05-22T12:00:00Z',
        },
      ],
    });

    const el = await recentActivityWidget.render(ownerCtx('7d'), 'compact');
    expect(el).not.toBeNull();
    render(el as React.ReactElement);

    // The raw key stays addressable via the title tooltip...
    const label = screen.getByTitle('vehicle_destruction');
    // ...but the visible label is humanised — no snake_case underscores.
    expect(label.textContent ?? '').not.toContain('_');
    expect(label.textContent?.trim()).toBeTruthy();
  });
});

describe('recentActivityWidget low-signal filter', () => {
  const ts = '2026-09-06T14:21:46.264Z';
  const ev = (event_type: string, payload: Record<string, unknown> = {}) => ({
    seq: 1,
    event_type,
    event_timestamp: ts,
    payload: { type: event_type, timestamp: ts, ...payload },
  });

  beforeEach(() => {
    vi.clearAllMocks();
  });

  it.each([
    'attachment_received',
    'planet_terrain_load',
    'location_inventory_requested',
    'shop_flow_response',
  ])('%s is instrumentation, not activity', (type) => {
    expect(isLowSignal(ev(type))).toBe(true);
  });

  it('keeps a HUD banner only when it belongs to a mission', () => {
    expect(isLowSignal(ev('hud_notification', { text: 'Entering Armistice Zone', mission_id: null }))).toBe(true);
    expect(isLowSignal(ev('hud_notification', { text: 'Package delivered', mission_id: 'm1' }))).toBe(false);
  });

  it('keeps the loadout burst and drops the bursts that summarise noise', () => {
    expect(isLowSignal(ev('burst_summary', { rule_id: 'loadout_restore_burst', size: 14 }))).toBe(false);
    expect(isLowSignal(ev('burst_summary', { rule_id: 'terrain_load_burst', size: 40 }))).toBe(true);
    expect(isLowSignal(ev('burst_summary', { rule_id: 'hud_notification_burst', size: 5 }))).toBe(true);
  });

  it.each(['join_pu', 'change_server', 'quantum_target_selected', 'seed_solar_system', 'resolve_spawn'])(
    '%s (movement noise) is low-signal here too, so a raw server page filters the same way',
    (type) => {
      expect(isLowSignal(ev(type))).toBe(true);
    },
  );

  it.each(['vehicle_stowed', 'player_death', 'mission_objective', 'session_end', 'shop_buy_request'])(
    '%s is activity',
    (type) => {
      expect(isLowSignal(ev(type))).toBe(false);
    },
  );

  it('load drops low-signal rows and asks for a page big enough to survive the cut', async () => {
    (listEvents as ReturnType<typeof vi.fn>).mockResolvedValue({
      events: [ev('attachment_received'), ev('vehicle_stowed'), ev('planet_terrain_load')],
      next_after: null,
    });
    const data = (await recentActivityWidget.load!(ownerCtx('7d'))) as RecentActivityData | null;
    expect(data?.events.map((e) => e.event_type)).toEqual(['vehicle_stowed']);
    const call = (listEvents as ReturnType<typeof vi.fn>).mock.calls[0] as [string, { limit?: number }];
    // Roughly 70% of a real stream is filtered here, so 20 rows would leave
    // the pane with a handful.
    expect(call[1].limit).toBeGreaterThanOrEqual(100);
  });

  it('load returns null when everything fetched was low-signal', async () => {
    (listEvents as ReturnType<typeof vi.fn>).mockResolvedValue({
      events: [ev('attachment_received')],
      next_after: null,
    });
    expect(await recentActivityWidget.load!(ownerCtx('7d'))).toBeNull();
  });
});
