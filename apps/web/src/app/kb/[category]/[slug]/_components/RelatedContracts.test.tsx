import React from 'react';
import { describe, it, expect, vi } from 'vitest';
import { render, screen } from '@testing-library/react';

vi.mock('next/link', () => ({
  default: ({ href, children }: { href: string; children: React.ReactNode }) => (
    <a href={String(href)}>{children}</a>
  ),
}));

import { RelatedContracts } from './RelatedContracts';
import type { ContractSummary } from '@/lib/contracts';

function summary(o: Partial<ContractSummary> = {}): ContractSummary {
  return {
    canonical_id: 'c1',
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
    ...o,
  } as ContractSummary;
}

describe('RelatedContracts', () => {
  it('lists each contract, linking to its catalogue entry', () => {
    render(
      <RelatedContracts
        contracts={[summary({ canonical_id: 'refuel_1', display_name: 'Refuel Run' })]}
      />,
    );
    expect(screen.getByText('Contracts')).toBeInTheDocument();
    expect(screen.getByRole('link', { name: /Refuel Run/ })).toHaveAttribute(
      'href',
      '/contracts/refuel_1',
    );
  });

  it('renders nothing at all when no contract references the entity', () => {
    // Most KB entries are referenced by nothing; a permanent empty
    // heading would be noise on every one of them.
    const { container } = render(<RelatedContracts contracts={[]} />);
    expect(container).toBeEmptyDOMElement();
    expect(screen.queryByText('Contracts')).toBeNull();
  });

  it('shows what distinguishes two contracts sharing a name', () => {
    // display_name is non-unique by design, so both rows must render AND
    // be tellable apart.
    render(
      <RelatedContracts
        contracts={[
          summary({ canonical_id: 'a', display_name: 'REFUEL REQUEST', first_step_location: 'asteroid mining base', step_count: 6 }),
          summary({ canonical_id: 'b', display_name: 'REFUEL REQUEST', first_step_location: 'client vessel', step_count: 4 }),
        ]}
      />,
    );
    expect(screen.getAllByText('REFUEL REQUEST')).toHaveLength(2);
    expect(screen.getByText(/asteroid mining base/)).toBeInTheDocument();
    expect(screen.getByText(/client vessel/)).toBeInTheDocument();
  });

  it('falls back to the canonical id when a contract has no name', () => {
    render(<RelatedContracts contracts={[summary({ canonical_id: 'no_name_x' })]} />);
    expect(screen.getByRole('link', { name: /no_name_x/ })).toBeInTheDocument();
  });
});
