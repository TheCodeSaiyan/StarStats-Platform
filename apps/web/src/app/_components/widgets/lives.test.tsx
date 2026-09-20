import React from 'react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen } from '@testing-library/react';

vi.mock('@/lib/api', () => ({
  getLives: vi.fn(),
}));

import { getLives, type LivesResponse } from '@/lib/api';
import { livesWidget } from './lives';
import { DEFAULT_SHARE_SCOPES, type ViewerCtx } from './types';

function ownerCtx(isOwner = true): ViewerCtx {
  return {
    ownerHandle: 'alice',
    viewerHandle: isOwner ? 'alice' : 'bob',
    isOwner,
    token: 'tok',
    shareScopes: { ...DEFAULT_SHARE_SCOPES },
    recipientScopes: null,
    range: '30d',
  };
}

function fixture(overrides: Partial<LivesResponse> = {}): LivesResponse {
  return {
    total_lives: 5,
    deaths: 4,
    mean_life_secs: 600,
    longest_life_secs: 5400,
    sessions: 3,
    deaths_per_session: 1.3,
    lives_ended_by_crash: 1,
    recent_lives: [],
    ...overrides,
  };
}

const mockLives = () => getLives as ReturnType<typeof vi.fn>;

describe('livesWidget', () => {
  beforeEach(() => { vi.clearAllMocks(); });

  it('is not range-aware and owner-only', async () => {
    // Range-aware now (lifetime + range split view).
    expect(livesWidget.rangeAware ?? false).toBe(true);
    expect(await livesWidget.isAvailable(ownerCtx(true))).toBe(true);
    expect(await livesWidget.isAvailable(ownerCtx(false))).toBe(false);
  });

  it('renders the headline readouts from a LivesResponse', async () => {
    mockLives().mockResolvedValue(fixture());
    const node = await livesWidget.render(ownerCtx(), 'compact');
    const { container } = render(node as React.ReactElement);
    // longest_life_secs=5400 -> 1h 30m, mean_life_secs=600 -> 10m
    expect(container.textContent).toContain('1h 30m');
    expect(container.textContent).toContain('10m');
    expect(container.textContent).toContain('1.3');
  });

  it('humanizes a multi-day survival streak', async () => {
    mockLives().mockResolvedValue(fixture({ longest_life_secs: 90_000 }));
    const node = await livesWidget.render(ownerCtx(), 'compact');
    const { container } = render(node as React.ReactElement);
    expect(container.textContent).toContain('1d 1h');
  });

  it('shows em dashes when streak/mean/deaths-per-session are null', async () => {
    mockLives().mockResolvedValue(
      fixture({ longest_life_secs: null, mean_life_secs: null, deaths_per_session: null }),
    );
    const node = await livesWidget.render(ownerCtx(), 'compact');
    const { container } = render(node as React.ReactElement);
    expect(container.textContent?.match(/—/g)?.length).toBe(3);
  });

  it('returns null (no data) when total_lives is 0', async () => {
    mockLives().mockResolvedValue(fixture({ total_lives: 0 }));
    expect(await livesWidget.render(ownerCtx(), 'compact')).toBeNull();
  });

  it('returns null when the fetch rejects', async () => {
    mockLives().mockRejectedValue(new Error('boom'));
    expect(await livesWidget.render(ownerCtx(), 'compact')).toBeNull();
  });
});

describe('lives widget death provenance', () => {
  beforeEach(() => {
    mockLives().mockReset();
  });

  it('always says deaths are reconstructed, because they always are', async () => {
    // This replaces two tests that asserted a "N of M inferred" marker gated
    // on `deaths_inferred > 0`. Nothing in the pipeline ever writes the
    // `body_class = "inferred"` value that counter keyed on, so the split was
    // structurally always zero and the marker never rendered — while EVERY
    // modern death is corpse-derived, CIG having removed the Actor Death
    // lines. The caveat was missing exactly where it applied.
    mockLives().mockResolvedValue(fixture({ deaths: 4 }));
    render(<>{await livesWidget.render(ownerCtx(), 'expanded')}</>);

    expect(
      screen.getByRole('button', { name: /how deaths is calculated/i }),
      'the provenance of a derived figure must travel with it, unconditionally',
    ).toBeInTheDocument();
    expect(screen.getByText('4')).toBeInTheDocument();
  });
});
