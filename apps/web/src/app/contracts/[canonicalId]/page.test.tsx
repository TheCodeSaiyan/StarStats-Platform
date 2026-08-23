/**
 * Contract detail page — rendering gates.
 *
 * These exist because `formatAdditionalRewards` being correct proves
 * nothing about whether the page shows its output. The non-aUEC awards
 * this covers were dropped silently at three separate layers before
 * reaching a screen (the server wire model, the merge, the review UI),
 * and each time the layer below was correct and unit-tested.
 */

import React from 'react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen } from '@testing-library/react';

// Keep the pure formatters real — the point is to exercise the page's
// wiring to them, not to restate their unit tests.
/**
 * The page reads the session and the calibration for its CHROME — it is a
 * public route and nothing on it is gated on either — and it renders a
 * projection surface, whose chrome and crumb navigate. All three need stubbing
 * to render this server component directly: `cookies()` throws outside a
 * request scope, and `useRouter` needs the app router mounted.
 *
 * A `vi.mock` factory REPLACES the module, so `next/navigation` goes through
 * the shared helper rather than a partial mock.
 */
vi.mock('@/lib/session', () => ({ getSession: async () => null }));
vi.mock('@/lib/theme', () => ({ getTheme: async () => 'terra' }));
vi.mock('next/navigation', async () => {
  const m = await import('@/test-support/next-navigation');
  return m.navigationMock();
});

vi.mock('@/lib/contracts', async (importActual) => {
  const actual = await importActual<typeof import('@/lib/contracts')>();
  return { ...actual, getContractDetail: vi.fn() };
});

// The locations catalogue is a filesystem read; an empty catalog makes
// EntityLink fall back to plain text, which is all these assertions need.
// Kept mocked so an accidental re-introduction of a catalogue fetch is
// observable rather than a silent filesystem read.
vi.mock('@/lib/reference', () => ({
  getCategoryBundle: vi.fn().mockResolvedValue({ catalog: new Map() }),
}));

import { getContractDetail, type ContractDetail } from '@/lib/contracts';
import { getCategoryBundle } from '@/lib/reference';
import ContractDetailPage from './page';

const mockGetContractDetail = getContractDetail as ReturnType<typeof vi.fn>;

type Reward = ContractDetail['contract']['reward'];

function mockContract(reward: Partial<Reward>, requirements: string[] = []): void {
  mockGetContractDetail.mockResolvedValue({
    kind: 'ok',
    contract: {
      canonical_id: 'combat_gauntlet_1',
      schema_version: '1',
      suggested_action: null,
      contract: {
        display_name: 'Combat Gauntlet - Scenario #1',
        reward: { amount: null, currency: null, bonus_amount: null, additional: [], ...reward },
        requirements,
        fees: [],
        attributes: [],
        primary_objectives: [],
        timeframe: {},
      },
      steps: [],
      first_seen_at: '2026-01-01T00:00:00Z',
      updated_at: '2026-01-01T00:00:00Z',
    },
  });
}

async function renderPage() {
  render(
    await ContractDetailPage({
      params: Promise.resolve({ canonicalId: 'combat_gauntlet_1' }),
    }),
  );
}

describe('ContractDetailPage additional rewards', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('shows the count and unit alongside the aUEC reward', async () => {
    mockContract({
      amount: 23000,
      currency: 'aUEC',
      additional: [
        { amount: 14, unit: 'MG Scrip', note: 'Completing this contract awards 14 MG Scrip.' },
      ],
    });
    await renderPage();

    // aUEC renders twice by design (hero readout + Reward section).
    expect(screen.getAllByText('23,000 aUEC').length).toBeGreaterThan(0);
    expect(screen.getByText('14 MG Scrip')).toBeInTheDocument();
  });

  it('opens the Reward section for a contract that pays ONLY scrip', async () => {
    // The section used to be gated on the aUEC amount alone, so an award
    // with no aUEC figure would render nothing at all.
    mockContract({
      amount: null,
      additional: [{ amount: 5, unit: 'Council Scrip', note: null }],
    });
    await renderPage();

    expect(screen.getByText('Reward')).toBeInTheDocument();
    expect(screen.getByText('5 Council Scrip')).toBeInTheDocument();
  });

  it('shows an award that states no count', async () => {
    mockContract({ amount: 100, additional: [{ amount: null, unit: 'MG Scrip', note: null }] });
    await renderPage();

    expect(screen.getByText('MG Scrip')).toBeInTheDocument();
  });

  it('never renders the verbatim sentence an award was read from', async () => {
    // Public output carries facts and authored guidance, never mission
    // prose. The server strips `note` from the projection; this pins that
    // the page does not surface it even if a note reaches it anyway —
    // defence at both layers, since the page is the last one.
    const PROSE = 'successful completion will net the holder an award of 1 MG Scrip';
    mockContract({
      amount: 23000,
      currency: 'aUEC',
      additional: [{ amount: 1, unit: 'MG Scrip', note: PROSE }],
    });
    const { container } = render(
      await ContractDetailPage({
        params: Promise.resolve({ canonicalId: 'combat_gauntlet_1' }),
      }),
    );

    // The award itself must still render — stripping prose must not
    // strip the fact.
    expect(screen.getByText('1 MG Scrip')).toBeInTheDocument();
    // The sentence must appear nowhere: not as text, not as an attribute.
    expect(container.innerHTML).not.toContain('net the holder');
    expect(screen.queryByTitle(PROSE)).toBeNull();
  });

  it('renders no additional-reward entries when there are none', async () => {
    mockContract({ amount: 23000, currency: 'aUEC', additional: [] });
    await renderPage();

    expect(screen.getAllByText('23,000 aUEC').length).toBeGreaterThan(0);
    expect(screen.queryByText(/Scrip/)).not.toBeInTheDocument();
  });
});

describe('ContractDetailPage requirements', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('renders requirements read from the description', async () => {
    mockContract({ amount: 23000, currency: 'aUEC' }, ['Tractor beam required']);
    await renderPage();
    expect(screen.getByText('Requirements')).toBeInTheDocument();
    expect(screen.getByText('Tractor beam required')).toBeInTheDocument();
  });

  it('omits the section when the contract states no prerequisites', async () => {
    // Most contracts have none; a permanent empty heading would be noise
    // on nearly every page.
    mockContract({ amount: 23000, currency: 'aUEC' }, []);
    await renderPage();
    expect(screen.queryByText('Requirements')).toBeNull();
  });
});

describe('ContractDetailPage KB entity links', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  function mockWithEntities(
    entities: Array<{
      kind: string;
      raw_value: string;
      ref_slug: string | null;
      ref_category: string | null;
      ref_match_count?: number;
    }>,
    steps: unknown[] = [],
  ): void {
    mockGetContractDetail.mockResolvedValue({
      kind: 'ok',
      contract: {
        canonical_id: 'c1',
        schema_version: '1',
        suggested_action: null,
        contract: {
          display_name: 'Salvage Run',
          requirements: [],
          reward: { amount: null, currency: null, bonus_amount: null, additional: [] },
          fees: [],
          attributes: [],
          primary_objectives: [],
          timeframe: {},
        },
        steps,
        entities,
        first_seen_at: '2026-01-01T00:00:00Z',
        updated_at: '2026-01-01T00:00:00Z',
      },
    });
  }

  it('links a resolved entity into the KB', async () => {
    mockWithEntities(
      [{ kind: 'location', raw_value: 'microTech', ref_slug: 'microtech', ref_category: 'location' }],
      [{ order: 1, summary: 'Go', entities: [{ kind: 'location', name: 'microTech' }] }],
    );
    await renderPage();

    expect(screen.getByRole('link', { name: 'microTech' })).toHaveAttribute(
      'href',
      '/kb/location/microtech',
    );
  });

  it('renders an unresolved entity as plain text, never a broken link', async () => {
    // Resolution is unambiguous-exact only. A guess would send people to
    // the wrong entry — worse than no link and harder to notice.
    mockWithEntities(
      [{ kind: 'location', raw_value: 'Somewhere Unmapped', ref_slug: null, ref_category: null }],
      [{ order: 1, summary: 'Go', entities: [{ kind: 'location', name: 'Somewhere Unmapped' }] }],
    );
    await renderPage();

    expect(screen.getByText('Somewhere Unmapped')).toBeInTheDocument();
    expect(screen.queryByRole('link', { name: 'Somewhere Unmapped' })).toBeNull();
  });

  it('keeps the descriptive step location as plain text', async () => {
    // "Caterpillar wreck site near microTech" names nothing the KB
    // holds; the canonical names live in `entities`.
    mockWithEntities(
      [{ kind: 'location', raw_value: 'microTech', ref_slug: 'microtech', ref_category: 'location' }],
      [
        {
          order: 1,
          summary: 'Salvage',
          location: 'Caterpillar wreck site near microTech',
          entities: [{ kind: 'location', name: 'microTech' }],
        },
      ],
    );
    await renderPage();

    expect(screen.getByText(/Caterpillar wreck site near microTech/)).toBeInTheDocument();
    expect(
      screen.queryByRole('link', { name: /Caterpillar wreck site/ }),
    ).toBeNull();
  });

  it('sends an ambiguous entity to a KB search rather than guessing one', async () => {
    // The registry holds genuine duplicates — "Sunset Berries" exists
    // three times — so refusing to link at all was the strict rule doing
    // the wrong thing. A search cannot assert a wrong identity.
    mockWithEntities(
      [{ kind: 'item', raw_value: 'Sunset Berries', ref_slug: null,
         ref_category: null, ref_match_count: 3 }],
      [{ order: 1, summary: 'Fetch', entities: [{ kind: 'item', name: 'Sunset Berries' }] }],
    );
    await renderPage();

    const link = screen.getByRole('link', { name: 'Sunset Berries' });
    expect(link.getAttribute('href')).toContain('/kb/item?');
    expect(link.getAttribute('href')).toContain('q=Sunset+Berries');
  });

  it('leaves an entity the KB knows nothing about as plain text', async () => {
    // A search we have not confirmed has results is a dead end; linking
    // to one anyway would be the guess this rule exists to prevent.
    mockWithEntities(
      [{ kind: 'location', raw_value: 'Nowhere At All', ref_slug: null,
         ref_category: null, ref_match_count: 0 }],
      [{ order: 1, summary: 'Go', entities: [{ kind: 'location', name: 'Nowhere At All' }] }],
    );
    await renderPage();

    expect(screen.getByText('Nowhere At All')).toBeInTheDocument();
    expect(screen.queryByRole('link', { name: 'Nowhere At All' })).toBeNull();
  });

  it('does not fetch a reference catalogue while rendering', async () => {
    // The vehicles bundle is ~4 MB. A per-render fetch is a real
    // regression and is invisible without this assertion.
    mockWithEntities([]);
    await renderPage();
    expect(getCategoryBundle).not.toHaveBeenCalled();
  });
});
