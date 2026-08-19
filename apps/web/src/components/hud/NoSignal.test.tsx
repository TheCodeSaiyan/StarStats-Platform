import React from 'react';
import { describe, it, expect } from 'vitest';
import { render, screen } from '@testing-library/react';
import { NoSignal } from './NoSignal';

describe('NoSignal', () => {
  it('renders the default title and the signal-lost graphic', () => {
    render(<NoSignal />);
    expect(screen.getByText('No Telemetry Signal Found')).toBeInTheDocument();
    expect(screen.getByRole('img', { name: /signal lost/i })).toBeInTheDocument();
  });

  it('shows the no-data hint by default', () => {
    render(<NoSignal />);
    expect(screen.getByText(/no activity recorded/i)).toBeInTheDocument();
  });

  it('distinguishes missing telemetry from an empty window', () => {
    render(<NoSignal reason="no-telemetry" />);
    expect(screen.getByText(/didn.t write this telemetry/i)).toBeInTheDocument();
  });

  it('honours a custom title and hint', () => {
    render(<NoSignal title="No fleet data" hint="Fly a ship to populate this." />);
    expect(screen.getByText('No fleet data')).toBeInTheDocument();
    expect(screen.getByText('Fly a ship to populate this.')).toBeInTheDocument();
  });

  it('suppresses the hint when passed an empty string', () => {
    render(<NoSignal hint="" />);
    // only the title text node, no hint element
    expect(screen.queryByText(/no activity recorded/i)).toBeNull();
  });

  it('applies the compact modifier class', () => {
    const { container } = render(<NoSignal compact />);
    expect(container.querySelector('.hud-nosignal--compact')).not.toBeNull();
  });
});
