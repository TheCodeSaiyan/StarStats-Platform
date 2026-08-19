import React from 'react';
import { describe, expect, it } from 'vitest';
import { render, screen } from '@testing-library/react';

import { TierChip } from './TierChip';

describe('TierChip', () => {
  it('renders the tier label alone when no subtype', () => {
    render(<TierChip tier="landmark" />);
    expect(screen.getByText('Landmark')).toBeInTheDocument();
  });

  it('renders tier · subtype in the full variant', () => {
    render(<TierChip tier="landmark" subtype="drug_lab" />);
    expect(screen.getByText('Landmark')).toBeInTheDocument();
    expect(screen.getByText('Drug lab')).toBeInTheDocument();
  });

  it('renders only the subtype in compact variant when subtype present', () => {
    render(<TierChip tier="landing_zone" subtype="city" compact />);
    expect(screen.getByText('City')).toBeInTheDocument();
    // Tier label is suppressed in compact when a subtype exists.
    expect(screen.queryByText('Landing zone')).not.toBeInTheDocument();
  });

  it('falls back to the tier label in compact when no subtype', () => {
    render(<TierChip tier="flotilla" compact />);
    expect(screen.getByText('Flotilla')).toBeInTheDocument();
  });

  it('forwards unknown subtype strings via subtypeLabel forward-compat', () => {
    // Wiki may add a sub-bucket the TS union doesn't know yet — the
    // chip must still render something readable.
    render(<TierChip tier="landmark" subtype="geothermal_vent" />);
    expect(screen.getByText('Geothermal Vent')).toBeInTheDocument();
  });
});
