import React from 'react';
import { describe, it, expect } from 'vitest';
import { render, screen } from '@testing-library/react';
import { DiscoverProfileCard } from './DiscoverProfileCard';

describe('DiscoverProfileCard', () => {
  it('renders a dense hud-tile link with handle + testid + data-handle', () => {
    render(
      <DiscoverProfileCard
        profile={{
          handle: 'Alice',
          display_name: 'Ali',
          supporter: null,
          last_active_at: null,
        }}
      />,
    );
    const card = screen.getByTestId('discover-profile-card');
    expect(card.className).toContain('hud-tile');
    expect(card.getAttribute('data-handle')).toBe('Alice');
    expect(card.getAttribute('href')).toBe('/u/Alice?source=discover');
    expect(screen.getByText('Alice')).toBeInTheDocument();
    expect(screen.getByText('Ali')).toBeInTheDocument();
  });

  it('renders the Active relative line when last_active_at is provided', () => {
    render(
      <DiscoverProfileCard
        profile={{
          handle: 'Bob',
          display_name: null,
          supporter: null,
          last_active_at: new Date(Date.now() - 3600 * 1000).toISOString(),
        }}
      />,
    );
    const card = screen.getByTestId('discover-profile-card');
    expect(card.getAttribute('data-handle')).toBe('Bob');
    expect(screen.getByText(/^Active /)).toBeInTheDocument();
  });

  it('omits display_name and active line when both are null', () => {
    render(
      <DiscoverProfileCard
        profile={{
          handle: 'Carol',
          display_name: null,
          supporter: null,
          last_active_at: null,
        }}
      />,
    );
    expect(screen.getByTestId('discover-profile-card')).toBeInTheDocument();
    expect(screen.queryByText(/^Active /)).toBeNull();
  });

  it('encodes special characters in the handle href', () => {
    render(
      <DiscoverProfileCard
        profile={{
          handle: 'han solo',
          display_name: null,
          supporter: null,
          last_active_at: null,
        }}
      />,
    );
    const card = screen.getByTestId('discover-profile-card');
    expect(card.getAttribute('href')).toBe('/u/han%20solo?source=discover');
  });
});
