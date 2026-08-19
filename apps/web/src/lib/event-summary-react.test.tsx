import React from 'react';
import { describe, expect, it, vi } from 'vitest';
import { render, screen } from '@testing-library/react';

import { renderEventSummary } from './event-summary-react';

// next/link needs the App Router context to render in isolation; in
// jsdom we only care that it emits an anchor with the right href, so
// stub it to a plain <a>.
vi.mock('next/link', () => ({
  default: ({
    href,
    children,
  }: {
    href: string;
    children: React.ReactNode;
  }) => <a href={String(href)}>{children}</a>,
}));

describe('renderEventSummary resolved_location', () => {
  it('links a fuzzy-resolved location even when the catalog has no entry', () => {
    // Empty catalogs (default) → the exact lookup misses. The tray's
    // resolved_location is the only thing that can produce a link.
    const payload = {
      type: 'planet_terrain_load',
      timestamp: '2026-06-03T00:00:00.000Z',
      planet: 'Stanton4a_RayariHydro_Kaltag',
    };
    const resolved = {
      display_name: 'Rayari Kaltag Research Outpost',
      slug: 'rayari-kaltag-research-outpost',
      system: 'Stanton',
    };

    render(
      <>{renderEventSummary(payload, undefined, undefined, resolved)}</>,
    );

    const link = screen.getByRole('link', {
      name: 'Rayari Kaltag Research Outpost',
    });
    expect(link).toHaveAttribute(
      'href',
      '/kb/location/rayari-kaltag-research-outpost',
    );
  });

  it('renders the resolved display name as plain text when it has no slug', () => {
    const payload = {
      type: 'player_death',
      timestamp: '2026-06-03T00:00:00.000Z',
      body_class: 'body_01',
      body_id: '1',
      zone: 'SomeUncataloguedZone',
    };
    const resolved = { display_name: 'Some Uncatalogued Zone', slug: null };

    render(
      <>{renderEventSummary(payload, undefined, undefined, resolved)}</>,
    );

    expect(screen.queryByRole('link')).not.toBeInTheDocument();
    expect(screen.getByText(/Some Uncatalogued Zone/)).toBeInTheDocument();
  });

  it('falls back to catalog/heuristic rendering when no resolved_location is passed', () => {
    // Regression guard: omitting resolved_location must not change the
    // pre-existing behaviour (no link from an empty catalog).
    const payload = {
      type: 'planet_terrain_load',
      timestamp: '2026-06-03T00:00:00.000Z',
      planet: 'Crusader',
    };

    render(<>{renderEventSummary(payload)}</>);

    expect(screen.queryByRole('link')).not.toBeInTheDocument();
    expect(screen.getByText(/Crusader/)).toBeInTheDocument();
  });
});
