import React from 'react';
import { describe, expect, it } from 'vitest';
import { render, screen } from '@testing-library/react';
import { SupporterChip } from './SupporterChip';
import type { SupporterStatusDto } from '@/lib/api';

function status(
  overrides: Partial<SupporterStatusDto>,
): SupporterStatusDto {
  return {
    state: 'none',
    name_plate: null,
    became_supporter_at: null,
    last_payment_at: null,
    grace_until: null,
    cancelled_at: null,
    current_tier_key: null,
    ...overrides,
  };
}

describe('SupporterChip', () => {
  it('renders nothing for a null status (no supporter row yet)', () => {
    const { container } = render(<SupporterChip status={null} />);
    expect(container.firstChild).toBeNull();
  });

  it('renders nothing for state="none"', () => {
    const { container } = render(
      <SupporterChip status={status({ state: 'none' })} />,
    );
    expect(container.firstChild).toBeNull();
  });

  it('renders an active coffee chip with the tier label', () => {
    render(
      <SupporterChip
        status={status({ state: 'active', current_tier_key: 'coffee' })}
      />,
    );
    expect(screen.getByText('Coffee supporter')).toBeInTheDocument();
  });

  it('renders an active standard chip', () => {
    render(
      <SupporterChip
        status={status({ state: 'active', current_tier_key: 'standard' })}
      />,
    );
    expect(screen.getByText('Supporter')).toBeInTheDocument();
  });

  it('renders an active generous chip', () => {
    render(
      <SupporterChip
        status={status({ state: 'active', current_tier_key: 'generous' })}
      />,
    );
    expect(screen.getByText('Generous supporter')).toBeInTheDocument();
  });

  it('appends the name plate when present', () => {
    render(
      <SupporterChip
        status={status({
          state: 'active',
          current_tier_key: 'coffee',
          name_plate: 'Caelum',
        })}
      />,
    );
    expect(screen.getByText(/Coffee supporter/)).toBeInTheDocument();
    expect(screen.getByText(/Caelum/)).toBeInTheDocument();
  });

  it('marks lapsed status visibly different (label includes "lapsed")', () => {
    render(
      <SupporterChip
        status={status({ state: 'lapsed', current_tier_key: 'standard' })}
      />,
    );
    expect(screen.getByText(/Supporter \(lapsed\)/)).toBeInTheDocument();
  });

  it('falls back to standard label when tier_key is unknown', () => {
    render(
      <SupporterChip
        status={status({
          state: 'active',
          current_tier_key: 'mystery-future-tier',
        })}
      />,
    );
    expect(screen.getByText('Supporter')).toBeInTheDocument();
  });

  it('exposes an accessible label combining tier + plate', () => {
    render(
      <SupporterChip
        status={status({
          state: 'active',
          current_tier_key: 'generous',
          name_plate: 'Caelum',
        })}
      />,
    );
    const chip = screen.getByRole('status');
    expect(chip).toHaveAttribute(
      'aria-label',
      'Generous supporter — Caelum',
    );
  });

  it('uses a tighter padding for size="sm"', () => {
    const { rerender } = render(
      <SupporterChip
        status={status({ state: 'active', current_tier_key: 'standard' })}
        size="sm"
      />,
    );
    const small = screen.getByRole('status') as HTMLElement;
    const smPadding = small.style.padding;

    rerender(
      <SupporterChip
        status={status({ state: 'active', current_tier_key: 'standard' })}
        size="md"
      />,
    );
    const md = screen.getByRole('status') as HTMLElement;
    expect(md.style.padding).not.toEqual(smPadding);
  });
});
