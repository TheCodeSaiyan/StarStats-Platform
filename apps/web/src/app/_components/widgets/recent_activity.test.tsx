import React from 'react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen } from '@testing-library/react';

vi.mock('@/lib/api', () => ({
  listEvents: vi.fn(),
}));

import { listEvents } from '@/lib/api';
import { recentActivityWidget } from './recent_activity';
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

describe('recentActivityWidget C2 owner-only gating', () => {
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

  it('is UNavailable to a visitor even with the recent_activity share scope on', () => {
    // /v1/me/events has no friend-scoped event-list equivalent, so the
    // widget must not render for a visitor (would show the viewer's events).
    expect(recentActivityWidget.isAvailable(visitorCtx)).toBe(false);
  });

  it('render returns null for a visitor without calling the me endpoint', async () => {
    const result = await recentActivityWidget.render(visitorCtx, 'compact');
    expect(result).toBeNull();
    expect(listEvents).not.toHaveBeenCalled();
  });
});

describe('recentActivityWidget H9 label formatting', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('renders a humanised label, never the raw snake_case event_type', async () => {
    (listEvents as ReturnType<typeof vi.fn>).mockResolvedValue({
      events: [
        {
          seq: 1,
          event_type: 'quantum_target_selected',
          event_timestamp: '2026-05-22T12:00:00Z',
        },
      ],
    });

    const el = await recentActivityWidget.render(ownerCtx('7d'), 'compact');
    expect(el).not.toBeNull();
    render(el as React.ReactElement);

    // The raw key stays addressable via the title tooltip...
    const label = screen.getByTitle('quantum_target_selected');
    // ...but the visible label is humanised — no snake_case underscores.
    expect(label.textContent ?? '').not.toContain('_');
    expect(label.textContent?.trim()).toBeTruthy();
  });
});
