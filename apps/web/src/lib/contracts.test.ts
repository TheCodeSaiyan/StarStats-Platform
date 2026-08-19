import { describe, it, expect } from 'vitest';
import {
  buildContractsQuery,
  isSearchRequest,
  formatReward,
  formatAdditionalRewards,
  contractNameHref,
  normalizeContractName,
  missionTimerBadge,
  distinctFacetValues,
  applyContractFacets,
  sortContractSummaries,
  type ContractSummary,
} from './contracts';

function mk(o: Partial<ContractSummary>): ContractSummary {
  return {
    canonical_id: 'id',
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
    first_seen_at: '2026-01-01T00:00:00Z',
    updated_at: '2026-01-01T00:00:00Z',
    ...o,
  } as ContractSummary;
}

describe('missionTimerBadge', () => {
  it('returns a Timed badge when the contract has a time limit', () => {
    const badge = missionTimerBadge({ has_time_limit: true });
    expect(badge).not.toBeNull();
    expect(badge?.label).toBe('Timed');
    expect(badge?.tone).toBeTruthy();
  });

  it('returns null when the contract explicitly has no time limit', () => {
    expect(missionTimerBadge({ has_time_limit: false })).toBeNull();
  });

  it('returns null when the timer state is unknown or absent', () => {
    expect(missionTimerBadge({ has_time_limit: null })).toBeNull();
    expect(missionTimerBadge({})).toBeNull();
    expect(missionTimerBadge(null)).toBeNull();
    expect(missionTimerBadge(undefined)).toBeNull();
  });
});

describe('buildContractsQuery', () => {
  it('omits empty and blank values', () => {
    expect(buildContractsQuery({ q: '', type: '  ', issuer: undefined })).toBe('');
  });

  it('maps params to the server querystring keys', () => {
    const qs = new URLSearchParams(
      buildContractsQuery({
        q: 'zane',
        type: 'bounty',
        issuer: 'Crusader',
        legalStatus: 'legal',
      }),
    );
    expect(qs.get('q')).toBe('zane');
    expect(qs.get('type')).toBe('bounty');
    expect(qs.get('issuer')).toBe('Crusader');
    expect(qs.get('legal_status')).toBe('legal');
  });

  it('drops offset=0 but keeps a positive offset', () => {
    expect(buildContractsQuery({ offset: 0 })).toBe('');
    expect(new URLSearchParams(buildContractsQuery({ offset: 48 })).get('offset')).toBe(
      '48',
    );
  });

  it('trims values', () => {
    expect(new URLSearchParams(buildContractsQuery({ q: '  glaciem  ' })).get('q')).toBe(
      'glaciem',
    );
  });

  it('emits every facet the server accepts', () => {
    const qs = buildContractsQuery({
      faction: 'Nine Tails',
      legalStatus: 'illegal',
      gameplayLoop: 'smuggling',
      type: 'Bounty Hunting',
      issuer: 'UEE',
      limit: 50,
    });
    const p = new URLSearchParams(qs);
    expect(p.get('faction')).toBe('Nine Tails');
    expect(p.get('legal_status')).toBe('illegal');
    expect(p.get('gameplay_loop')).toBe('smuggling');
    expect(p.get('type')).toBe('Bounty Hunting');
  });

  it('a facet-only request still reaches an endpoint that supports it', () => {
    // Pre-Task-8 this returned false, routing facet-only requests to
    // /api/contracts where faction/gameplay_loop did not exist. Both
    // endpoints now accept all three facets, so either route is correct
    // -- what must NOT happen is the param being dropped.
    const qs = buildContractsQuery({ faction: 'Nine Tails' });
    expect(new URLSearchParams(qs).get('faction')).toBe('Nine Tails');
  });
});

describe('isSearchRequest', () => {
  it('is true when q or location is present', () => {
    expect(isSearchRequest({ q: 'x' })).toBe(true);
    expect(isSearchRequest({ location: 'Glaciem' })).toBe(true);
  });
  it('is false for filter-only or empty params', () => {
    expect(isSearchRequest({ type: 'bounty' })).toBe(false);
    expect(isSearchRequest({ q: '   ' })).toBe(false);
    expect(isSearchRequest({})).toBe(false);
  });
});

describe('formatReward', () => {
  it('formats amount + currency with thousands separators', () => {
    expect(formatReward({ amount: 8500, currency: 'aUEC' })).toBe('8,500 aUEC');
  });
  it('appends a positive bonus', () => {
    expect(formatReward({ amount: 8500, currency: 'aUEC', bonus_amount: 1500 })).toBe(
      '8,500 aUEC (+1,500)',
    );
  });
  it('defaults currency to aUEC', () => {
    expect(formatReward({ amount: 100 })).toBe('100 aUEC');
  });
  it('returns null when there is no amount', () => {
    expect(formatReward({})).toBeNull();
    expect(formatReward({ bonus_amount: 500 })).toBeNull();
  });
});

describe('formatAdditionalRewards', () => {
  it('formats count + unit', () => {
    expect(
      formatAdditionalRewards({ additional: [{ amount: 14, unit: 'MG Scrip', note: null }] }),
    ).toEqual([{ text: '14 MG Scrip', note: null }]);
  });

  it('keeps an award that states no count', () => {
    // The prose often names an award without a number. Dropping these
    // would hide the only evidence the contract pays anything but aUEC.
    expect(
      formatAdditionalRewards({ additional: [{ amount: null, unit: 'Council Scrip' }] }),
    ).toEqual([{ text: 'Council Scrip', note: null }]);
  });

  it('carries the provenance note through', () => {
    expect(
      formatAdditionalRewards({
        additional: [{ amount: 14, unit: 'MG Scrip', note: 'awards 14 MG Scrip.' }],
      })[0].note,
    ).toBe('awards 14 MG Scrip.');
  });

  it('separates thousands', () => {
    expect(
      formatAdditionalRewards({ additional: [{ amount: 12500, unit: 'MG Scrip' }] })[0].text,
    ).toBe('12,500 MG Scrip');
  });

  it('drops entries with no unit — "14" alone is not a reward', () => {
    expect(
      formatAdditionalRewards({
        additional: [
          { amount: 14, unit: null },
          { amount: 1, unit: '   ' },
          { amount: 2, unit: 'MG Scrip' },
        ],
      }),
    ).toEqual([{ text: '2 MG Scrip', note: null }]);
  });

  it('returns an empty list for absent, empty, or malformed input', () => {
    expect(formatAdditionalRewards(null)).toEqual([]);
    expect(formatAdditionalRewards(undefined)).toEqual([]);
    expect(formatAdditionalRewards({})).toEqual([]);
    expect(formatAdditionalRewards({ additional: [] })).toEqual([]);
  });
});

describe('distinctFacetValues', () => {
  it('dedupes case-insensitively, drops blanks, sorts', () => {
    const rows = [
      mk({ contract_type: 'bounty' }),
      mk({ contract_type: 'Bounty' }),
      mk({ contract_type: 'delivery' }),
      mk({ contract_type: '' }),
      mk({ contract_type: null }),
    ];
    expect(distinctFacetValues(rows, 'contract_type')).toEqual(['bounty', 'delivery']);
  });

  it('works over the gameplay_loop facet', () => {
    const rows = [
      mk({ gameplay_loop: 'smuggling' }),
      mk({ gameplay_loop: 'Smuggling' }),
      mk({ gameplay_loop: 'combat' }),
      mk({ gameplay_loop: null }),
    ];
    expect(distinctFacetValues(rows, 'gameplay_loop')).toEqual(['combat', 'smuggling']);
  });
});

describe('applyContractFacets', () => {
  const rows = [
    mk({ canonical_id: 'a', contract_type: 'bounty', issuer: 'Crusader', legal_status: 'legal' }),
    mk({ canonical_id: 'b', contract_type: 'delivery', issuer: 'Hurston', legal_status: 'legal' }),
    mk({ canonical_id: 'c', contract_type: 'bounty', issuer: 'Hurston', legal_status: 'illegal' }),
  ];
  it('filters by a single facet (case-insensitive)', () => {
    expect(applyContractFacets(rows, { type: 'BOUNTY' }).map((r) => r.canonical_id)).toEqual(['a', 'c']);
  });
  it('composes multiple facets with AND', () => {
    expect(
      applyContractFacets(rows, { type: 'bounty', issuer: 'hurston' }).map((r) => r.canonical_id),
    ).toEqual(['c']);
  });
  it('blank filters match everything', () => {
    expect(applyContractFacets(rows, { type: '', issuer: undefined }).length).toBe(3);
  });

  it('filters by faction and gameplay_loop (the two newest facets)', () => {
    const withFactionAndLoop = [
      mk({ canonical_id: 'd', faction: 'Nine Tails', gameplay_loop: 'smuggling' }),
      mk({ canonical_id: 'e', faction: 'Nine Tails', gameplay_loop: 'combat' }),
      mk({ canonical_id: 'f', faction: 'UEE', gameplay_loop: 'smuggling' }),
    ];
    expect(
      applyContractFacets(withFactionAndLoop, { faction: 'nine tails' }).map((r) => r.canonical_id),
    ).toEqual(['d', 'e']);
    expect(
      applyContractFacets(withFactionAndLoop, { gameplayLoop: 'SMUGGLING' }).map((r) => r.canonical_id),
    ).toEqual(['d', 'f']);
    expect(
      applyContractFacets(withFactionAndLoop, {
        faction: 'Nine Tails',
        gameplayLoop: 'smuggling',
      }).map((r) => r.canonical_id),
    ).toEqual(['d']);
  });
});

describe('sortContractSummaries', () => {
  const rows = [
    mk({ canonical_id: 'z', display_name: 'Zeta', reward_amount: 100, updated_at: '2026-01-01T00:00:00Z' }),
    mk({ canonical_id: 'a', display_name: 'Alpha', reward_amount: null, updated_at: '2026-03-01T00:00:00Z' }),
    mk({ canonical_id: 'm', display_name: 'Mu', reward_amount: 5000, updated_at: '2026-02-01T00:00:00Z' }),
  ];
  it('name sorts A–Z by display name', () => {
    expect(sortContractSummaries(rows, 'name').map((r) => r.display_name)).toEqual(['Alpha', 'Mu', 'Zeta']);
  });
  it('reward sorts high→low with nulls last', () => {
    expect(sortContractSummaries(rows, 'reward').map((r) => r.canonical_id)).toEqual(['m', 'z', 'a']);
  });
  it('updated sorts newest-first', () => {
    expect(sortContractSummaries(rows, 'updated').map((r) => r.canonical_id)).toEqual(['a', 'm', 'z']);
  });
  it('does not mutate the input', () => {
    const before = rows.map((r) => r.canonical_id);
    sortContractSummaries(rows, 'name');
    expect(rows.map((r) => r.canonical_id)).toEqual(before);
  });
});

describe('contractNameHref', () => {
  it('links straight to the contract when exactly one carries the name', () => {
    expect(
      contractNameHref('Patrol Dangerous Sector', {
        name: 'patrol dangerous sector',
        match_count: 1,
        canonical_id: 'p1',
      }),
    ).toBe('/contracts/p1');
  });

  it('sends an ambiguous name to the filtered candidate list', () => {
    // display_name is non-unique by design; the list carries the
    // disambiguation row a person needs to pick the right one.
    const href = contractNameHref('Combat Gauntlet - Scenario #1', {
      name: 'combat gauntlet - scenario #1',
      match_count: 3,
      canonical_id: null,
    });
    expect(href).toBe('/contracts?q=Combat+Gauntlet+-+Scenario+%231');
  });

  it('never links to an id when the name is ambiguous', () => {
    // Guards against a future "best guess" regression: even if the
    // server sent an id alongside a count above one, it must be ignored.
    const href = contractNameHref('X', {
      name: 'x',
      match_count: 2,
      canonical_id: 'should-be-ignored',
    });
    expect(href).not.toContain('should-be-ignored');
    expect(href).toContain('/contracts?q=');
  });

  it('returns null when nothing matches, so the name stays plain text', () => {
    expect(contractNameHref('Unknown', { name: 'unknown', match_count: 0, canonical_id: null }))
      .toBeNull();
    expect(contractNameHref('Unknown', undefined)).toBeNull();
  });
});

describe('normalizeContractName', () => {
  it('matches the server normalization: trim, collapse, lowercase', () => {
    expect(normalizeContractName('  Combat   GAUNTLET  ')).toBe('combat gauntlet');
  });
});
