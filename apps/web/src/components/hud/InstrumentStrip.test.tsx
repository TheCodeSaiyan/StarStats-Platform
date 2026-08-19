import React from 'react';
import { describe, it, expect } from 'vitest';
import { render, screen } from '@testing-library/react';
import { InstrumentStrip } from './InstrumentStrip';

describe('InstrumentStrip', () => {
  it('renders title + inline readouts', () => {
    render(<InstrumentStrip title="@alice" readouts={[{ k: 'play', v: '1695h' }, { k: 'k/d', v: '2.4' }]} />);
    expect(screen.getByText('@alice')).toBeInTheDocument();
    expect(screen.getByText('1695h')).toBeInTheDocument();
    expect(screen.getByText('2.4')).toBeInTheDocument();
  });

  it('uses the compact title size by default', () => {
    const { container } = render(<InstrumentStrip title="X" />);
    const titleEl = container.querySelector('.hud-tile__title') as HTMLElement;
    expect(titleEl.style.fontSize).toBe('14px');
  });

  it('uses a larger (clamped) title size in the hero variant', () => {
    const { container } = render(<InstrumentStrip size="hero" title="Hero" />);
    const titleEl = container.querySelector('.hud-tile__title') as HTMLElement;
    expect(titleEl.style.fontSize).toContain('clamp');
  });
});
