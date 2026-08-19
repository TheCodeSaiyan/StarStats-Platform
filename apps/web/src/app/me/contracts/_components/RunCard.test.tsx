import React from 'react';
import { describe, it, expect, vi } from 'vitest';
import { render, screen } from '@testing-library/react';

vi.mock('next/link', () => ({
  default: ({ href, children }: { href: string; children: React.ReactNode }) => (
    <a href={String(href)}>{children}</a>
  ),
}));

import { RunCard } from './RunCard';
import type { ContractRunRow } from '@/lib/api';

function run(o: Partial<ContractRunRow> = {}): ContractRunRow {
  return {
    mission_id: 'm1',
    name: 'Patrol Dangerous Sector',
    state: 'completed',
    closed_by: 'hud_complete',
    step_count: 3,
    steps_complete: 3,
    steps_remaining: 0,
    partial_history: false,
    connected_server: null,
    accepted_at: null,
    closed_at: null,
    last_event_at: null,
    steps: [],
    ...o,
  } as ContractRunRow;
}

describe('RunCard contract-name link', () => {
  it('links the run name when the page resolved it', () => {
    render(<RunCard run={run()} href="/contracts/p1" />);
    expect(screen.getByRole('link', { name: 'Patrol Dangerous Sector' })).toHaveAttribute(
      'href',
      '/contracts/p1',
    );
  });

  it('renders the name as plain text when nothing resolved', () => {
    // An unmatched run name must not become a link to nowhere.
    render(<RunCard run={run({ name: 'Never Published' })} href={null} />);
    expect(screen.getByText('Never Published')).toBeInTheDocument();
    expect(screen.queryByRole('link', { name: 'Never Published' })).toBeNull();
  });

  it('renders as plain text when no href prop is passed at all', () => {
    render(<RunCard run={run({ name: 'No Prop' })} />);
    expect(screen.getByText('No Prop')).toBeInTheDocument();
    expect(screen.queryByRole('link', { name: 'No Prop' })).toBeNull();
  });

  it('an ambiguous-name href points at the candidate list, not a contract', () => {
    // The page passes whatever contractNameHref returned; this pins that
    // the card renders it verbatim rather than reshaping it into an id.
    render(<RunCard run={run()} href="/contracts?q=Patrol+Dangerous+Sector" />);
    const link = screen.getByRole('link', { name: 'Patrol Dangerous Sector' });
    expect(link.getAttribute('href')).toContain('/contracts?q=');
  });
});
