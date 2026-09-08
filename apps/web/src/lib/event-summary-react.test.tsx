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

describe('renderEventSummary: variants the tray emits that had no case', () => {
  const ts = '2026-09-06T14:04:15.205Z';

  it('quantum_route links the destination through the resolved location', () => {
    const resolved = { display_name: 'Pyro', slug: 'pyro', system: 'Pyro' };
    render(
      <>
        {renderEventSummary(
          {
            type: 'quantum_route',
            timestamp: ts,
            start_system: 'Stanton',
            destination: 'LOC_pyro',
            vehicle_class: 'RSI_Polaris',
            vehicle_id: '1',
          },
          undefined,
          undefined,
          resolved,
        )}
      </>,
    );
    expect(
      screen.getByRole('link', { name: 'Pyro' }).getAttribute('href'),
    ).toBe('/kb/location/pyro');
    expect(screen.getByText(/Route plotted/)).toBeTruthy();
  });

  it('location_changed links where the player now is', () => {
    const resolved = {
      display_name: 'Everus Harbor',
      slug: 'everus-harbor',
      system: 'Stanton',
    };
    render(
      <>
        {renderEventSummary(
          { type: 'location_changed', timestamp: ts, from: null, to: 'Stanton|hurston|everus' },
          undefined,
          undefined,
          resolved,
        )}
      </>,
    );
    expect(screen.getByRole('link', { name: 'Everus Harbor' })).toBeTruthy();
  });

  it('mission_objective states the outcome in words', () => {
    render(
      <>
        {renderEventSummary({
          type: 'mission_objective',
          timestamp: ts,
          objective_id: 'o',
          mission_id: null,
          state: 'failed',
          text: 'Hold the point',
        })}
      </>,
    );
    expect(screen.getByText('Objective failed: Hold the point')).toBeTruthy();
  });

  it('never renders a bare "<type> event" for a known variant', () => {
    const known = [
      { type: 'quantum_arrived', timestamp: ts, vehicle_class: 'RSI_Polaris', vehicle_id: '1' },
      { type: 'shop_request_timed_out', timestamp: ts, shop_id: null, item_class: null, timed_out_after_secs: 30 },
      { type: 'item_equip_change', timestamp: ts, action: 'equip', item_class: 'x', port: null, items_count: null },
      { type: 'mission_quantum_destination_selected', timestamp: ts, beacon_id: 'b', is_mission_destination: true, travel_confirmed: false },
      { type: 'travel_to_contract_location', timestamp: ts, beacon_id: 'b', travel_started: true, travel_completed: false },
    ];
    for (const p of known) {
      const { container, unmount } = render(<>{renderEventSummary(p)}</>);
      // The switch used to fall off the end for these and render NOTHING,
      // which is why an "is not `<type> event`" check alone is not enough.
      expect(container.textContent?.trim()).toBeTruthy();
      expect(container.textContent).not.toMatch(/_/);
      unmount();
    }
  });
});

describe('renderEventSummary: what the live log showed', () => {
  const ts = '2026-09-06T14:16:39.536Z';

  it('never prints a shop item that is only an engine GUID', () => {
    const { container } = render(
      <>
        {renderEventSummary({
          type: 'shop_buy_request',
          timestamp: ts,
          shop_id: '1',
          item_class: '72b91153-5a3e-4d71-af5c-f6c57ea2891a',
          quantity: null,
          raw: '',
        })}
      </>,
    );
    expect(container.textContent).toBe('Shop purchase requested');
  });

  it('describes a stow without the engine vehicle id', () => {
    const { container } = render(
      <>
        {renderEventSummary({
          type: 'vehicle_stowed',
          timestamp: ts,
          vehicle_id: '816633546929',
          landing_area: 'LandingArea_ShipElevator_HangarMediumFront',
          landing_area_id: '1',
          zone_host_id: '2',
        })}
      </>,
    );
    expect(container.textContent).toMatch(/^Stowed a ship at /);
    expect(container.textContent).not.toMatch(/816633546929/);
  });
});

describe('renderEventSummary: procedural landing pads', () => {
  it('drops the braced engine GUID a procedural pad name carries', () => {
    const { container } = render(
      <>
        {renderEventSummary({
          type: 'vehicle_stowed',
          timestamp: '2026-09-06T14:00:06.515Z',
          vehicle_id: '816082288190',
          landing_area: '[PROC]LandingArea_Pad_MedB-001_{4E778C54-496B-4CD9-9298-AC947973B4BB}',
          landing_area_id: '1',
          zone_host_id: '2',
        })}
      </>,
    );
    expect(container.textContent).toBe('Stowed a ship at Pad Med B-001');
  });
});
