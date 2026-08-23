import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen } from '@testing-library/react';

// Keep the rest of @/lib/contracts real (facet/sort/filter helpers are
// pure and exercised elsewhere in contracts.test.ts); only stub the
// network-backed listing calls so the page renders from fixtures.
/**
 * The page reads the session and the calibration for its CHROME — it is a
 * public route and nothing in the catalogue is gated on either. Both reach for
 * `cookies()`, which throws outside a request scope, so rendering this server
 * component directly in vitest needs them stubbed. Signed-OUT is the right
 * default here: it is the state a stranger browsing the catalogue is in.
 */
vi.mock('@/lib/session', () => ({ getSession: async () => null }));

// The page now renders a projection surface, whose chrome and crumb navigate —
// so `useRouter` has to exist. A `vi.mock` factory REPLACES the module, which
// is why this goes through the shared helper rather than a partial mock.
vi.mock('next/navigation', async () => {
  const m = await import('@/test-support/next-navigation');
  return m.navigationMock();
});
vi.mock('@/lib/theme', () => ({ getTheme: async () => 'terra' }));

vi.mock('@/lib/contracts', async (importActual) => {
  const actual = await importActual<typeof import('@/lib/contracts')>();
  return {
    ...actual,
    listContracts: vi.fn(),
    listAllContracts: vi.fn(),
  };
});

import { listAllContracts, type ContractSummary } from '@/lib/contracts';
import ContractsPage from './page';

const mockListAllContracts = listAllContracts as ReturnType<typeof vi.fn>;

function summary(overrides: Partial<ContractSummary> = {}): ContractSummary {
  return {
    canonical_id: 'contract-1',
    display_name: null,
    contract_type: null,
    subcategory: null,
    gameplay_loop: null,
    issuer: null,
    faction: null,
    legal_status: null,
    reward_amount: null,
    reward_currency: null,
    confidence_score: null,
    patch_version: null,
    first_step_location: null,
    required_item: null,
    step_count: null,
    first_seen_at: '2026-01-01T00:00:00Z',
    updated_at: '2026-01-01T00:00:00Z',
    ...overrides,
  } as ContractSummary;
}

function mockList(contracts: ContractSummary[]): void {
  mockListAllContracts.mockResolvedValue(contracts);
}

describe('ContractsPage catalog rows', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('shows what differs between same-named contracts', async () => {
    mockList([
      summary({
        canonical_id: 'a',
        display_name: 'REFUEL REQUEST: Aegis Avenger Stalker',
        first_step_location: 'asteroid mining base',
        reward_amount: 58750,
        reward_currency: 'aUEC',
        required_item: 'Hydrogen Fuel',
        patch_version: '4.2',
        step_count: 6,
      }),
      summary({
        canonical_id: 'b',
        display_name: 'REFUEL REQUEST: Aegis Avenger Stalker',
        first_step_location: 'client vessel',
        reward_amount: 150250,
        reward_currency: 'aUEC',
        required_item: null,
        patch_version: '4.2',
        step_count: 4,
      }),
    ]);

    render(await ContractsPage({ searchParams: Promise.resolve({}) }));

    // Both rows render — the name is intentionally non-unique and
    // neither may be hidden or merged.
    expect(
      screen.getAllByText(/REFUEL REQUEST: Aegis Avenger Stalker/),
    ).toHaveLength(2);
    // ...and the user can tell them apart.
    expect(screen.getByText(/asteroid mining base/)).toBeInTheDocument();
    expect(screen.getByText(/client vessel/)).toBeInTheDocument();
    expect(screen.getByText(/58,750/)).toBeInTheDocument();
    expect(screen.getByText(/150,250/)).toBeInTheDocument();
  });

  it('omits absent fields rather than rendering empty separators', async () => {
    mockList([
      summary({
        first_step_location: null,
        required_item: null,
        reward_amount: null,
        patch_version: null,
        step_count: null,
      }),
    ]);

    render(await ContractsPage({ searchParams: Promise.resolve({}) }));

    expect(screen.queryByText(/·\s*·/)).not.toBeInTheDocument();
    expect(screen.queryByText(/^@\s*$/)).not.toBeInTheDocument();
  });
});

describe('ContractsPage gameplay-loop facet', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  function contractsWithLoops(): ContractSummary[] {
    return [
      summary({ canonical_id: 'contract-smuggle', gameplay_loop: 'smuggling' }),
      summary({ canonical_id: 'contract-combat', gameplay_loop: 'combat' }),
    ];
  }

  it('renders a chip per distinct gameplay_loop value, linked with the facet in the URL', async () => {
    mockList(contractsWithLoops());

    render(await ContractsPage({ searchParams: Promise.resolve({}) }));

    expect(screen.getByRole('link', { name: 'combat' })).toHaveAttribute(
      'href',
      '/contracts?gameplay_loop=combat',
    );
    expect(screen.getByRole('link', { name: 'smuggling' })).toHaveAttribute(
      'href',
      '/contracts?gameplay_loop=smuggling',
    );
  });

  it('filters the rendered contracts when gameplay_loop is set via the URL', async () => {
    mockList(contractsWithLoops());

    render(
      await ContractsPage({
        searchParams: Promise.resolve({ gameplay_loop: 'smuggling' }),
      }),
    );

    expect(screen.getByRole('link', { name: 'contract-smuggle' })).toBeInTheDocument();
    expect(screen.queryByRole('link', { name: 'contract-combat' })).not.toBeInTheDocument();
  });

  it('makes the active gameplay_loop facet clearable without hand-editing the address bar', async () => {
    mockList(contractsWithLoops());

    render(
      await ContractsPage({
        searchParams: Promise.resolve({ gameplay_loop: 'smuggling' }),
      }),
    );

    const active = screen.getByRole('link', { name: 'smuggling' });
    expect(active).toHaveAttribute('data-active', 'true');
    // Clicking the already-active chip drops the param instead of resubmitting it.
    expect(active).toHaveAttribute('href', '/contracts');
  });
});
