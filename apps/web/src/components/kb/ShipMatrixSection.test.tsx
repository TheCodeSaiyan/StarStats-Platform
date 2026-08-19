import React from 'react';
import { afterEach, describe, expect, it } from 'vitest';
import { cleanup, render, screen } from '@testing-library/react';

import { ShipMatrixSection } from './ShipMatrixSection';
import { parseShipMatrix } from '@/lib/ship-matrix';

const SAMPLE = parseShipMatrix({
  specs: {
    length: 23.5,
    beam: 21.5,
    height: 6.5,
    mass: 226345,
    scm_speed: 215,
    afterburner_speed: 1275,
    min_crew: 1,
    max_crew: 2,
    cargo: 46,
  },
  production_status: 'flight-ready',
  description: 'A versatile medium freighter.',
  media: ['https://media.example/1.jpg', 'https://media.example/2.jpg'],
  matched_by: 'name',
  matched_at: '2026-06-12T00:00:00Z',
})!;

describe('ShipMatrixSection', () => {
  afterEach(() => {
    cleanup();
  });

  it('renders the specs grid from the parsed ship_matrix', () => {
    render(<ShipMatrixSection shipMatrix={SAMPLE} mediaUrls={[]} />);
    expect(screen.getByText('Length')).toBeInTheDocument();
    expect(screen.getByText('23.5 m')).toBeInTheDocument();
    expect(screen.getByText('SCM speed')).toBeInTheDocument();
    expect(screen.getByText('Crew')).toBeInTheDocument();
    expect(screen.getByText('1–2')).toBeInTheDocument();
    expect(screen.getByText('Cargo')).toBeInTheDocument();
    expect(screen.getByText('46 SCU')).toBeInTheDocument();
    expect(screen.getByText('Production status')).toBeInTheDocument();
  });

  it('renders the description block', () => {
    render(<ShipMatrixSection shipMatrix={SAMPLE} mediaUrls={[]} />);
    expect(
      screen.getByText('A versatile medium freighter.'),
    ).toBeInTheDocument();
  });

  it('always renders the CIG disclaimer when the section renders', () => {
    render(<ShipMatrixSection shipMatrix={SAMPLE} mediaUrls={[]} />);
    expect(
      screen.getByText(/© Cloud Imperium Rights LLC/),
    ).toBeInTheDocument();
  });

  it('renders the image gallery when media URLs are provided', () => {
    render(
      <ShipMatrixSection
        shipMatrix={SAMPLE}
        mediaUrls={[
          '/api/v1/reference/vehicles/AEGS_Avenger/media/0',
          '/api/v1/reference/vehicles/AEGS_Avenger/media/1',
        ]}
      />,
    );
    const imgs = screen.getAllByRole('img');
    expect(imgs).toHaveLength(2);
    expect(imgs[0]).toHaveAttribute(
      'src',
      '/api/v1/reference/vehicles/AEGS_Avenger/media/0',
    );
  });

  it('hides the gallery entirely when no media URLs are provided', () => {
    const { container } = render(
      <ShipMatrixSection shipMatrix={SAMPLE} mediaUrls={[]} />,
    );
    expect(screen.queryByRole('img')).toBeNull();
    // No gallery region at all when media is empty.
    expect(
      container.querySelector('[data-testid="ship-matrix-gallery"]'),
    ).toBeNull();
  });

  it('still renders specs + disclaimer when description is absent', () => {
    const noDesc = parseShipMatrix({ specs: { length: 10 } })!;
    render(<ShipMatrixSection shipMatrix={noDesc} mediaUrls={[]} />);
    expect(screen.getByText('Length')).toBeInTheDocument();
    expect(screen.getByText(/© Cloud Imperium Rights LLC/)).toBeInTheDocument();
  });
});
