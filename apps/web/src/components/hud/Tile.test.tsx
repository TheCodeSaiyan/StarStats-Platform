import React from 'react';
import { describe, it, expect } from 'vitest';
import { render, screen } from '@testing-library/react';
import { Tile } from './Tile';

describe('Tile', () => {
  it('renders eyebrow, title, substat, and body', () => {
    render(<Tile span={2} eyebrow="TRAVEL" title="Travel" substat="22 jumps"><p>body-content</p></Tile>);
    expect(screen.getByText('Travel')).toBeInTheDocument();
    expect(screen.getByText('22 jumps')).toBeInTheDocument();
    expect(screen.getByText('body-content')).toBeInTheDocument();
  });
  it('applies the column span', () => {
    const { container } = render(<Tile span={3} title="X">x</Tile>);
    expect((container.firstChild as HTMLElement).style.gridColumn).toBe('span 3');
  });
  it('renders a compact one-line empty state instead of body when empty', () => {
    render(<Tile span={1} title="Stability" empty="— no crashes in range —"><p>should-not-show</p></Tile>);
    expect(screen.getByText('— no crashes in range —')).toBeInTheDocument();
    expect(screen.queryByText('should-not-show')).not.toBeInTheDocument();
  });
});
