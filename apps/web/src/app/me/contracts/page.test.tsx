import React from 'react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';

/**
 * Render the page and open the Runs group.
 *
 * `/me/contracts` is a projection now: its sections are grouped behind the
 * lens rail and only the active group is MOUNTED, so the run list is not in
 * the tree until Runs is selected. Every assertion below is about a run, so
 * they all go through here. The figures and the wording are unchanged — this
 * only navigates to them.
 */
async function renderRuns(): Promise<void> {
  render(await ContractsPage({}));
  const runsTab = screen
    .getAllByRole('button')
    .find((b) => b.textContent === 'Runs');
  if (!runsTab) throw new Error('Runs group not found in the lens rail');
  fireEvent.click(runsTab);
}

// Mock next/navigation (redirect) and next/link. `redirect` throws, same
// as the real implementation, so a signed-out render actually halts
// instead of falling through to code that assumes a session exists.
vi.mock('next/navigation', async () => {
  const m = await import('@/test-support/next-navigation');
  return m.navigationMock();
});

vi.mock('next/link', () => ({
  default: ({
    href,
    children,
  }: {
    href: string;
    children: React.ReactNode;
  }) => <a href={String(href)}>{children}</a>,
}));

// Mock session
vi.mock('@/lib/session', () => ({
  getSession: vi.fn(),
}));

// Mock api functions
vi.mock('@/lib/api', () => ({
  getContracts: vi.fn(),
  statusOf: vi.fn(() => undefined),
}));

import { redirect } from 'next/navigation';
import { getSession } from '@/lib/session';
import { getContracts } from '@/lib/api';
import type { ContractRunRow, ContractStepRow, ContractsResponse } from '@/lib/api';
import ContractsPage from './page';

const mockRedirect = redirect as unknown as ReturnType<typeof vi.fn>;
const mockGetSession = getSession as ReturnType<typeof vi.fn>;
const mockGetContracts = getContracts as ReturnType<typeof vi.fn>;

function makeStep(overrides: Partial<ContractStepRow> = {}): ContractStepRow {
  return {
    order: 0,
    state: 'complete',
    text: 'Return to the contract giver',
    objective_id: 'obj1',
    started_at: null,
    completed_at: null,
    ...overrides,
  };
}

function makeRun(overrides: Partial<ContractRunRow> = {}): ContractRunRow {
  return {
    mission_id: 'm1',
    name: 'Combat Gauntlet - Scenario #5',
    state: 'completed',
    closed_by: 'hud_complete',
    step_count: 2,
    steps_complete: 2,
    steps_remaining: 0,
    partial_history: false,
    connected_server: null,
    accepted_at: '2026-07-20T10:00:00Z',
    closed_at: '2026-07-20T10:30:00Z',
    last_event_at: '2026-07-20T10:30:00Z',
    steps: [],
    ...overrides,
  };
}

function makeResponse(runs: ContractRunRow[]): ContractsResponse {
  const completed = runs.filter((r) => r.state === 'completed').length;
  const failed = runs.filter((r) => r.state === 'failed').length;
  const abandoned = runs.filter((r) => r.state === 'abandoned').length;
  const in_progress = runs.filter((r) => r.state === 'in_progress').length;
  const withdrawn = runs.filter((r) => r.state === 'withdrawn').length;
  const unknown = runs.filter((r) => r.state === 'unknown').length;
  const resolved = completed + failed + abandoned;
  return {
    completed,
    failed,
    abandoned,
    in_progress,
    withdrawn,
    unknown,
    total: runs.length,
    completion_pct: resolved > 0 ? Math.round((completed / resolved) * 100) : null,
    runs,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  mockGetSession.mockResolvedValue({ token: 'test-token', claimedHandle: 'pilot' });
});

describe('ContractsPage', () => {
  it('redirects signed-out visitors instead of rendering their own data', async () => {
    mockGetSession.mockResolvedValue(null);
    await expect(ContractsPage({})).rejects.toThrow('REDIRECT:/auth/login?next=/me/contracts');
    expect(mockRedirect).toHaveBeenCalledWith('/auth/login?next=/me/contracts');
  });

  it('requests include_steps and renders each step\'s text verbatim', async () => {
    const run = makeRun({
      steps: [
        makeStep({ order: 0, text: 'Go to a debris field above Euterpe: ' }),
        makeStep({ order: 1, text: 'Return to the contract giver' }),
      ],
    });
    mockGetContracts.mockResolvedValue(makeResponse([run]));

    await renderRuns();

    // Default range is 7d -> 168 hours (see @/lib/range's DEFAULT_RANGE).
    expect(mockGetContracts).toHaveBeenCalledWith('test-token', 24 * 7, true);
    // Regex, not an exact string: the trailing ": " is intentional (the
    // game's own banner wording, passed through verbatim) but testing-
    // library's default text matcher trims trailing whitespace, so an
    // exact-string query for it never matches.
    expect(screen.getByText(/Go to a debris field above Euterpe:\s*$/)).toBeInTheDocument();
    expect(screen.getByText('Return to the contract giver')).toBeInTheDocument();
  });

  it('renders closed_by as human text, never the raw enum value', async () => {
    const run = makeRun({ state: 'abandoned', closed_by: 'session_end' });
    mockGetContracts.mockResolvedValue(makeResponse([run]));

    await renderRuns();

    expect(screen.getByText(/app exit/)).toBeInTheDocument();
    expect(screen.queryByText('session_end')).not.toBeInTheDocument();
  });

  it('falls back to the objective id when step text is null', async () => {
    const run = makeRun({
      steps: [makeStep({ order: 0, state: 'in_progress', text: null, objective_id: 'obj-fallback' })],
    });
    mockGetContracts.mockResolvedValue(makeResponse([run]));

    await renderRuns();

    expect(screen.getByText('obj-fallback')).toBeInTheDocument();
  });

  it('surfaces the partial_history warning', async () => {
    const run = makeRun({ partial_history: true });
    mockGetContracts.mockResolvedValue(makeResponse([run]));

    await renderRuns();

    expect(screen.getByText(/history incomplete/)).toBeInTheDocument();
  });

  it('caps rendered runs at 200 and reports how many were dropped', async () => {
    const runs = Array.from({ length: 210 }, (_, i) =>
      makeRun({
        mission_id: `m${i}`,
        name: `Run ${i}`,
        accepted_at: new Date(2026, 0, 1, 0, 0, i).toISOString(),
      }),
    );
    mockGetContracts.mockResolvedValue(makeResponse(runs));

    await renderRuns();

    expect(screen.getByText(/200 most recent runs of 210/)).toBeInTheDocument();
    expect(screen.getByText(/10 older runs/)).toBeInTheDocument();
    // ...and that the 200 kept are the NEWEST. The counts above hold even
    // if `byAcceptedDesc` inverts — 200 rendered and 10 dropped either
    // way — so without these the cap could silently keep the oldest runs
    // and show the user a page that claims to be "most recent".
    expect(screen.getByText('Run 209')).toBeInTheDocument();
    expect(screen.queryByText('Run 0')).not.toBeInTheDocument();
  });

  it('renders a no-signal state when there are no runs in the window', async () => {
    mockGetContracts.mockResolvedValue(makeResponse([]));
    await renderRuns();
    expect(screen.getByText(/No signal in this window/i)).toBeInTheDocument();
  });

  it('renders a friendly error state when the fetch fails', async () => {
    mockGetContracts.mockRejectedValue(new Error('boom'));
    render(await ContractsPage({}));
    expect(screen.getByText(/couldn.t load contract history/i)).toBeInTheDocument();
  });
});
