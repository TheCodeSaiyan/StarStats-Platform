import React from 'react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen } from '@testing-library/react';

// Mock next/navigation (redirect). `redirect` throws, same as the real
// implementation, so a signed-out/non-moderator render actually halts
// instead of falling through to code that assumes a session exists —
// see `/me/contracts/page.test.tsx` for the same convention.
vi.mock('next/navigation', () => ({
  redirect: vi.fn((url: string) => {
    throw new Error(`REDIRECT:${url}`);
  }),
}));

// Mock session
vi.mock('@/lib/session', () => ({
  getSession: vi.fn(),
}));

// Mock api functions
vi.mock('@/lib/api', () => ({
  getAdminContractGaps: vi.fn(),
  ApiCallError: class ApiCallError extends Error {
    status: number;
    constructor(status: number, message: string) {
      super(message);
      this.status = status;
    }
  },
}));

import { redirect } from 'next/navigation';
import { getSession } from '@/lib/session';
import { ApiCallError, getAdminContractGaps } from '@/lib/api';
import type { ContractCatalogGapsResponse, ContractGapDto } from '@/lib/api';
import ContractGapsPage from './page';

const mockRedirect = redirect as unknown as ReturnType<typeof vi.fn>;
const mockGetSession = getSession as ReturnType<typeof vi.fn>;
const mockGetAdminContractGaps = getAdminContractGaps as ReturnType<
  typeof vi.fn
>;

function mockSession(
  session: { token: string; staffRoles: string[] } | null,
) {
  mockGetSession.mockResolvedValue(session);
}

function mockGaps(response: ContractCatalogGapsResponse) {
  mockGetAdminContractGaps.mockResolvedValue(response);
}

function makeGap(overrides: Partial<ContractGapDto> = {}): ContractGapDto {
  return {
    name: 'Some Contract',
    run_count: 1,
    distinct_handles: 1,
    first_seen: '2026-06-01T00:00:00Z',
    last_seen: '2026-07-01T00:00:00Z',
    ...overrides,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  mockSession({ token: 'test-token', staffRoles: ['moderator'] });
});

describe('ContractGapsPage', () => {
  // Fixture order deliberately differs from alphabetical order ("A Call
  // to Arms" < "Combat Gauntlet..."): if the component re-sorted by name
  // this test would still pass under a name-only ORDER BY, proving
  // nothing. Ranking by run_count (occurrence) is the entire point —
  // Combat Gauntlet is a small slice of distinct gap names but the
  // largest slice of runs, and a name-sorted list buries that win.
  it('ranks by run count and states the coverage impact', async () => {
    mockGaps({
      gaps: [
        makeGap({
          name: 'Combat Gauntlet - Scenario #5',
          run_count: 64,
          distinct_handles: 12,
        }),
        makeGap({
          name: 'A Call to Arms',
          run_count: 36,
          distinct_handles: 9,
        }),
      ],
      total_unmatched_runs: 280,
    });

    render(await ContractGapsPage());

    const rows = screen.getAllByRole('row').slice(1); // drop header row
    expect(rows[0]).toHaveTextContent('Combat Gauntlet - Scenario #5');
    expect(rows[1]).toHaveTextContent('A Call to Arms');
    // Header line states what publishing would gain, not just a list.
    expect(screen.getByText(/280/)).toBeInTheDocument();
  });

  it('redirects a signed-out visitor to login', async () => {
    mockSession(null);
    await expect(ContractGapsPage()).rejects.toThrow(
      /REDIRECT:\/auth\/login\?next=\/admin\/contract-gaps/,
    );
    expect(mockRedirect).toHaveBeenCalledWith(
      '/auth/login?next=/admin/contract-gaps',
    );
  });

  // The case the test above does NOT cover: signed in (a real session
  // exists) but the server's RequireModerator gate rejects the request
  // with 403 because the caller lacks the role. Distinct failure mode
  // from being signed out entirely, and page.tsx routes it to a
  // different redirect target (/me, not /auth/login).
  it('redirects a non-moderator visitor to /me', async () => {
    mockGetAdminContractGaps.mockRejectedValue(
      new ApiCallError(403, { error: 'forbidden' }),
    );
    await expect(ContractGapsPage()).rejects.toThrow(/REDIRECT:\/me/);
    expect(mockRedirect).toHaveBeenCalledWith('/me');
  });

  it('renders a dash for null first_seen/last_seen instead of Invalid Date', async () => {
    mockGaps({
      gaps: [
        makeGap({
          name: 'A Call to Arms',
          first_seen: null,
          last_seen: null,
        }),
      ],
      total_unmatched_runs: 36,
    });

    render(await ContractGapsPage());

    expect(screen.queryByText(/Invalid Date/i)).not.toBeInTheDocument();
    expect(screen.getAllByText('—').length).toBeGreaterThanOrEqual(2);
  });
});
