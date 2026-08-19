import React from 'react';
import { afterEach } from 'vitest';
import { cleanup, render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { StatusBadge } from './StatusBadge';

afterEach(() => {
  cleanup();
});

describe('StatusBadge', () => {
  it('renders the kebab-case status as a friendly label', () => {
    render(<StatusBadge status="in-design" />);
    expect(screen.getByText('In design')).toBeInTheDocument();
  });

  it('capitalises a single-word status', () => {
    render(<StatusBadge status="shipped" />);
    expect(screen.getByText('Shipped')).toBeInTheDocument();
  });

  it('exposes the raw status as a data attribute for styling hooks', () => {
    const { container } = render(<StatusBadge status="parked" />);
    const badge = container.querySelector('[data-status="parked"]');
    expect(badge).not.toBeNull();
  });

  it('falls back to capitalised label for unrecognised statuses', () => {
    render(<StatusBadge status="unknown-state" />);
    // First char uppercased; rest preserved.
    expect(screen.getByText('Unknown-state')).toBeInTheDocument();
  });
});
