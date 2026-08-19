import React from 'react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render } from '@testing-library/react';

vi.mock('@/lib/api', () => ({
  getPlayerFacts: vi.fn(),
}));

import { getPlayerFacts } from '@/lib/api';
import { factsWidget } from './facts';
import { DEFAULT_SHARE_SCOPES } from './types';
import type { ViewerCtx } from './types';

const ownerCtx: ViewerCtx = {
  ownerHandle: 'alice',
  viewerHandle: 'alice',
  isOwner: true,
  token: 'tok',
  shareScopes: { ...DEFAULT_SHARE_SCOPES },
  recipientScopes: null,
  range: '7d',
};

const visitorCtx: ViewerCtx = {
  ...ownerCtx,
  viewerHandle: 'bob',
  isOwner: false,
  token: 'bob-tok',
};

function fact(id: string, headline: string, detail: string) {
  return {
    id,
    scope: 'lifetime',
    headline,
    detail,
    evidence: { value: 1, baseline: 2, sample_size: 40, unit: 'count' },
    provenance: 'session durations',
  };
}

function payload(over: Record<string, unknown> = {}) {
  return {
    facts: [
      fact('playtime_concentration', 'Half your time came from just 4 of 60 flights', '4 of 60 sessions account for half your 2d 4h total'),
      fact('weekly_pace', "You're flying 3h 20m a week lately", '13h 20m in the last 30 days'),
    ],
    enough_history: true,
    sessions_considered: 60,
    sessions_required: 8,
    ...over,
  };
}

const mocked = getPlayerFacts as unknown as ReturnType<typeof vi.fn>;

beforeEach(() => {
  mocked.mockReset();
});

describe('facts widget', () => {
  it('renders the strongest fact when compact', async () => {
    mocked.mockResolvedValue(payload());

    const body = await factsWidget.render(ownerCtx, 'compact');
    const { container } = render(<>{body}</>);

    expect(container.textContent).toContain('Half your time came from just 4 of 60 flights');
    // Compact shows ONE claim; the rest are signposted, not rendered.
    expect(container.textContent).not.toContain("You're flying 3h 20m a week lately");
    expect(container.textContent).toContain('1 more in the expanded view');
  });

  it('shows each fact with its arithmetic when expanded', async () => {
    mocked.mockResolvedValue(payload());

    const body = await factsWidget.render(ownerCtx, 'expanded');
    const { container } = render(<>{body}</>);

    expect(container.textContent).toContain('Half your time came from just 4 of 60 flights');
    expect(container.textContent).toContain("You're flying 3h 20m a week lately");
    // The detail line is what lets a claim be audited rather than trusted.
    expect(container.textContent).toContain('4 of 60 sessions account for half');
  });

  it('tells a new player why it is empty instead of claiming no activity', async () => {
    // "Too new" and "no activity" are different answers. The canvas's
    // generic empty copy would assert the latter, which is simply untrue
    // for someone who has flown three times.
    mocked.mockResolvedValue(
      payload({ facts: [], enough_history: false, sessions_considered: 3 }),
    );

    const body = await factsWidget.render(ownerCtx, 'compact');
    const { container } = render(<>{body}</>);

    expect(container.textContent).toContain('Not enough flight time yet');
    expect(container.textContent).toContain('8 sessions');
    expect(container.textContent).toContain('you have 3');
  });

  it('distinguishes "nothing stands out" from "too new"', async () => {
    // Enough history, but no rule cleared its own sample gate.
    mocked.mockResolvedValue(payload({ facts: [] }));

    const body = await factsWidget.render(ownerCtx, 'compact');
    const { container } = render(<>{body}</>);

    expect(container.textContent).toContain('Nothing stands out yet');
    expect(container.textContent).not.toContain('Not enough flight time');
  });

  it('never fetches me-scoped facts for a visitor', async () => {
    const body = await factsWidget.render(visitorCtx, 'compact');

    expect(body).toBeNull();
    expect(mocked).not.toHaveBeenCalled();
  });

  it('degrades to nothing when the endpoint fails', async () => {
    mocked.mockRejectedValue(new Error('boom'));

    const body = await factsWidget.render(ownerCtx, 'compact');

    expect(body).toBeNull();
  });

  it('is not range-aware — scope belongs to each fact', () => {
    // Re-scoping a lifetime observation to the dashboard range is the
    // defect that made the commerce and corridor widgets quietly wrong.
    expect(factsWidget.rangeAware).toBe(false);
  });
});
