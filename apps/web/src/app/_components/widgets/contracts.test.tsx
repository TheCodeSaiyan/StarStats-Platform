import React from 'react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen } from '@testing-library/react';

// Mock next/link so it renders a plain <a> in jsdom
vi.mock('next/link', () => ({
  default: ({
    href,
    children,
  }: {
    href: string;
    children: React.ReactNode;
  }) => <a href={String(href)}>{children}</a>,
}));

vi.mock('@/lib/api', () => ({ getContracts: vi.fn() }));

// Stub only the network call; contractNameHref stays REAL so the tier
// rule is exercised rather than restated.
vi.mock('@/lib/contracts', async (importActual) => {
  const actual = await importActual<typeof import('@/lib/contracts')>();
  return { ...actual, resolveContractNames: vi.fn().mockResolvedValue(new Map()) };
});

import { getContracts } from '@/lib/api';
import { resolveContractNames } from '@/lib/contracts';
import { contractsWidget } from './contracts';
import { DEFAULT_SHARE_SCOPES } from './types';
import type { ViewerCtx } from './types';

function ownerCtx(): ViewerCtx {
  return {
    ownerHandle: 'alice',
    viewerHandle: 'alice',
    isOwner: true,
    token: 'tok',
    shareScopes: { ...DEFAULT_SHARE_SCOPES },
    recipientScopes: null,
    range: '90d',
  };
}

describe('contractsWidget', () => {
  beforeEach(() => {
    vi.mocked(getContracts).mockReset();
    vi.mocked(resolveContractNames).mockReset();
    vi.mocked(resolveContractNames).mockResolvedValue(new Map());
  });

  /** Two runs of one contract, so `topName` is deterministic. */
  function twoRunsOf(name: string) {
    return {
      completed: 2, failed: 0, abandoned: 0, in_progress: 0,
      withdrawn: 0, unknown: 0, total: 2, completion_pct: 100,
      runs: [1, 2].map((i) => ({
        mission_id: `m${i}`, name, state: 'completed',
        closed_by: 'hud_complete', step_count: 1, steps_complete: 1,
        steps_remaining: 0, partial_history: false, steps: [],
      })),
    };
  }

  it('links the most-run contract to its catalogue entry when unique', async () => {
    vi.mocked(getContracts).mockResolvedValue(twoRunsOf('Patrol Dangerous Sector') as never);
    vi.mocked(resolveContractNames).mockResolvedValue(
      new Map([['patrol dangerous sector',
        { name: 'patrol dangerous sector', match_count: 1, canonical_id: 'p1' }]]),
    );

    render(<>{await contractsWidget.render(ownerCtx(), 'expanded')}</>);
    expect(screen.getByRole('link', { name: 'Patrol Dangerous Sector' }))
      .toHaveAttribute('href', '/contracts/p1');
  });

  it('sends an ambiguous most-run name to the candidate list', async () => {
    // display_name is non-unique by design — linking to one of four
    // would be a confident wrong answer.
    vi.mocked(getContracts).mockResolvedValue(
      twoRunsOf('Combat Gauntlet - Scenario #1') as never,
    );
    vi.mocked(resolveContractNames).mockResolvedValue(
      new Map([['combat gauntlet - scenario #1',
        { name: 'combat gauntlet - scenario #1', match_count: 4, canonical_id: null }]]),
    );

    render(<>{await contractsWidget.render(ownerCtx(), 'expanded')}</>);
    const link = screen.getByRole('link', { name: 'Combat Gauntlet - Scenario #1' });
    expect(link.getAttribute('href')).toContain('/contracts?q=');
  });

  it('leaves an unmatched most-run name as plain text', async () => {
    vi.mocked(getContracts).mockResolvedValue(twoRunsOf('Never Published') as never);
    render(<>{await contractsWidget.render(ownerCtx(), 'expanded')}</>);
    expect(screen.getByText('Never Published')).toBeInTheDocument();
    expect(screen.queryByRole('link', { name: 'Never Published' })).toBeNull();
  });

  it('renders the completion headline, meter and secondary counts', async () => {
    vi.mocked(getContracts).mockResolvedValue({
      completed: 8, failed: 1, abandoned: 3, in_progress: 2,
      withdrawn: 0, unknown: 0, total: 14, completion_pct: 67,
      runs: [
        { mission_id: 'm1', name: 'Combat Gauntlet - Scenario #5', state: 'completed',
          closed_by: 'hud_complete', step_count: 3, steps_complete: 3,
          steps_remaining: 0, partial_history: false, steps: [] },
        { mission_id: 'm2', name: 'Combat Gauntlet - Scenario #5', state: 'abandoned',
          closed_by: 'session_end', step_count: 2, steps_complete: 1,
          steps_remaining: 1, partial_history: false, steps: [] },
      ],
    });

    const node = await contractsWidget.render(ownerCtx(), 'expanded');
    const { container } = render(<>{node}</>);

    // resolved = completed(8) + failed(1) + abandoned(3) = 12 — the SAME
    // denominator `rate` uses (67%). The API's `total` (14) also counts
    // 2 in-progress runs; asserting on it here would have hidden the
    // done/rate mismatch this test guards against.
    expect(screen.getByText('8/12')).toBeDefined();   // done
    expect(screen.queryByText('8/14')).toBeNull();
    expect(screen.getByText('67%')).toBeDefined();    // rate
    // Query the readout containing the label, not a bare value: two
    // secondary readouts can share a count and a label-less assertion
    // wouldn't catch the labels being swapped.
    const abandonedReadout = screen.getByText('abandoned').closest('.hud-readout');
    expect(abandonedReadout?.textContent).toContain('3');
    // Most-run contract, from `runs`.
    expect(screen.getByText(/Combat Gauntlet - Scenario #5/)).toBeDefined();
    const fill = container.querySelector('.hud-meter__fill');
    expect(fill).not.toBeNull();
    expect(fill?.getAttribute('style')).toContain('67%');
    // The tile's drill-down link to the contract history page.
    const link = screen.getByText('View contract history →');
    expect(link.closest('a')?.getAttribute('href')).toBe('/me/contracts');
    // NEGATIVE SPACE — the tile must NOT opt into `include_steps`. It reads
    // only run-level counts, and every `/me` load renders it; opting in
    // would ship the full per-step objective text (~237 KB on a real
    // account) to a tile that never displays a single step. The server
    // default is regression-covered on its own side
    // (`stats_contracts_include_steps_flag_gates_step_population`); this
    // asserts the *client* still declines it, which no other test does.
    expect(vi.mocked(getContracts).mock.calls[0][2]).toBeFalsy();
  });

  it('renders nothing when the user has no contract runs', async () => {
    vi.mocked(getContracts).mockResolvedValue({
      completed: 0, failed: 0, abandoned: 0, in_progress: 0,
      withdrawn: 0, unknown: 0, total: 0, completion_pct: null, runs: [],
    });
    const node = await contractsWidget.render(ownerCtx(), 'expanded');
    expect(node).toBeNull();
  });

  it('excludes superseded runs from the "most run" count', async () => {
    vi.mocked(getContracts).mockResolvedValue({
      completed: 2, failed: 0, abandoned: 0, in_progress: 0,
      withdrawn: 0, unknown: 0, total: 2, completion_pct: 100,
      runs: [
        // Three superseded rows for the same mission — outnumbers "Real
        // Contract" 3-to-2 and would win the "most run" tile if
        // superseded runs were counted, but a superseded run is
        // re-accept bookkeeping, not an outcome the headline recognizes.
        { mission_id: 'm1', name: 'Reaccepted Contract', state: 'superseded',
          closed_by: 'superseded', step_count: 1, steps_complete: 0,
          steps_remaining: 1, partial_history: false, steps: [] },
        { mission_id: 'm1', name: 'Reaccepted Contract', state: 'superseded',
          closed_by: 'superseded', step_count: 1, steps_complete: 0,
          steps_remaining: 1, partial_history: false, steps: [] },
        { mission_id: 'm1', name: 'Reaccepted Contract', state: 'superseded',
          closed_by: 'superseded', step_count: 1, steps_complete: 0,
          steps_remaining: 1, partial_history: false, steps: [] },
        { mission_id: 'm2', name: 'Real Contract', state: 'completed',
          closed_by: 'hud_complete', step_count: 2, steps_complete: 2,
          steps_remaining: 0, partial_history: false, steps: [] },
        { mission_id: 'm3', name: 'Real Contract', state: 'completed',
          closed_by: 'hud_complete', step_count: 2, steps_complete: 2,
          steps_remaining: 0, partial_history: false, steps: [] },
      ],
    });

    const node = await contractsWidget.render(ownerCtx(), 'expanded');
    render(<>{node}</>);

    expect(screen.getByText(/Real Contract/)).toBeDefined();
    expect(screen.queryByText(/Reaccepted Contract/)).toBeNull();
  });

  it('renders an em dash (not 0%) when nothing has resolved yet', async () => {
    vi.mocked(getContracts).mockResolvedValue({
      completed: 0, failed: 0, abandoned: 0, in_progress: 4,
      withdrawn: 0, unknown: 0, total: 4, completion_pct: null,
      runs: [
        // `none`, not `''` — an unclosed run's `closed_by` is the string
        // "none" on the wire (see `closed_by_str`).
        { mission_id: 'm1', name: 'Ongoing Run', state: 'in_progress',
          closed_by: 'none', step_count: 2, steps_complete: 1,
          steps_remaining: 1, partial_history: false, steps: [] },
      ],
    });

    const node = await contractsWidget.render(ownerCtx(), 'expanded');
    const { container } = render(<>{node}</>);

    expect(screen.getByText('—')).toBeDefined();
    expect(screen.queryByText('0%')).toBeNull();
    // The meter itself still renders, pinned to 0 — not skipped, not
    // left at a stale value.
    const fill = container.querySelector('.hud-meter__fill');
    expect(fill).not.toBeNull();
    expect(fill?.getAttribute('style')).toContain('0%');
  });
});
