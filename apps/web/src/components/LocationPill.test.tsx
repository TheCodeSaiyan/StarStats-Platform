/**
 * Tests for LocationPill and LocationChip headline logic.
 *
 * Core invariant: the raw shard id is NEVER the headline. When a
 * `ResolvedLocation` has only a `shard` and no `city` or `planet`,
 * both components must fall through to "In transit". The shard still
 * appears in the subline / subtext rendered by `buildSubline`.
 */

import React from 'react';
import { describe, it, expect, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import { LocationPill, LocationChip } from './LocationPill';
import type { ResolvedLocation } from '@/lib/api';

// EntityLink brings in next/link and hover-card deps that require a
// full Next.js runtime. Mock it to a plain <span> so we can test
// the headline-selection logic in isolation.
vi.mock('@/components/kb/EntityLink', () => ({
  EntityLink: ({
    label,
    classKey,
  }: {
    label?: string;
    classKey?: string | null;
  }) => <span data-testid="entity-link">{label ?? classKey ?? ''}</span>,
}));

/** Minimal ResolvedLocation with a known real place. */
function makeLocation(
  overrides: Partial<ResolvedLocation> = {},
): ResolvedLocation {
  return {
    city: null,
    planet: null,
    system: null,
    shard: null,
    last_seen_at: new Date(Date.now() - 5 * 60_000).toISOString(),
    source_event_type: 'planet_terrain_load',
    entered_at: null,
    entered_at_is_lower_bound: false,
    raw_city_key: null,
    raw_planet_key: null,
    resolved_location: null,
    ...overrides,
  };
}

// ---------------------------------------------------------------------------
// LocationPill
// ---------------------------------------------------------------------------

describe('LocationPill headline', () => {
  it('shows the city when city is present', () => {
    render(
      <LocationPill
        location={makeLocation({ city: 'Orison', planet: 'Crusader', system: 'Stanton' })}
      />,
    );
    expect(screen.getByTestId('entity-link').textContent).toBe('Orison');
  });

  it('shows the planet when city is absent', () => {
    render(
      <LocationPill
        location={makeLocation({ planet: 'Daymar', system: 'Stanton' })}
      />,
    );
    expect(screen.getByTestId('entity-link').textContent).toBe('Daymar');
  });

  it('shows "In transit" — NOT the shard — when only shard is present', () => {
    // This is the core invariant: a shard id (e.g. "pub_euw1b_test") is a
    // server routing identifier, never a human-readable place name. When
    // city and planet are both absent the headline falls to "In transit".
    render(
      <LocationPill
        location={makeLocation({ shard: 'pub_euw1b_test' })}
      />,
    );
    expect(screen.getByTestId('entity-link').textContent).toBe('In transit');
    // The shard must still appear somewhere — verify it is in the DOM
    // (buildSubline adds "Shard pub_euw1b_test" to the subline).
    expect(screen.getByText(/pub_euw1b_test/)).toBeInTheDocument();
  });

  it('shows the shard in the subline when city is also present', () => {
    render(
      <LocationPill
        location={makeLocation({
          city: 'Orison',
          planet: 'Crusader',
          system: 'Stanton',
          shard: 'pub_euw1b_test',
        })}
      />,
    );
    // Headline is the city
    expect(screen.getByTestId('entity-link').textContent).toBe('Orison');
    // Shard in the subline
    expect(screen.getByText(/shard pub_euw1b_test/i)).toBeInTheDocument();
  });

  it('renders null when location is null', () => {
    const { container } = render(<LocationPill location={null} />);
    expect(container.firstChild).toBeNull();
  });
});

// ---------------------------------------------------------------------------
// LocationChip
// ---------------------------------------------------------------------------

describe('LocationChip headline', () => {
  it('shows the city when city is present', () => {
    render(
      <LocationChip
        location={makeLocation({ city: 'Lorville', planet: 'Hurston', system: 'Stanton' })}
      />,
    );
    // The chip renders multiple EntityLink elements (headline + sub).
    // The first one is the headline.
    const links = screen.getAllByTestId('entity-link');
    expect(links[0].textContent).toBe('Lorville');
  });

  it('shows "In transit" — NOT the shard — when only shard is present', () => {
    render(
      <LocationChip
        location={makeLocation({ shard: 'pub_euw1b_test' })}
      />,
    );
    const links = screen.getAllByTestId('entity-link');
    expect(links[0].textContent).toBe('In transit');
  });

  it('renders null when location is null', () => {
    const { container } = render(<LocationChip location={null} />);
    expect(container.firstChild).toBeNull();
  });
});
