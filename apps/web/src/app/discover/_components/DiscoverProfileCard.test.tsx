import React from 'react';
import { describe, it, expect } from 'vitest';
import { render, screen } from '@testing-library/react';
import { DiscoverProfileCard } from './DiscoverProfileCard';

describe('DiscoverProfileCard', () => {
  // `hp-rw hp-rw--text` — the system's own METER-ROW shape. `Directory.jsx`
  // draws pilots through `CatalogueLayout`'s ranked list state, not the entity
  // grid the catalogue uses for ships; the first pass at this page invented a
  // card because that kit screen was never opened. `--text` is the meterless
  // variant, because the listing endpoint carries nothing countable to rank by
  // and a share bar would be inventing the ranking it displays.
  //
  // The testid, the `data-handle` and the `?source=discover` href are UNCHANGED
  // — three specs assert the listing through them.
  it('renders a directory entry link with handle + testid + data-handle', () => {
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
    expect(card.className).toContain('hp-rw');
    expect(card.className).toContain('hp-rw--text');
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
