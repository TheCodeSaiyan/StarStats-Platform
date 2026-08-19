// vitest uses the classic JSX runtime here — the explicit React import
// is required or JSX-rendering tests 500 with "React is not defined".
import React from 'react';
import { describe, it, expect } from 'vitest';
import { render } from '@testing-library/react';
import { Sparkline } from './Sparkline';

describe('Sparkline', () => {
  it('renders an accessible inline SVG for a multi-point series', () => {
    const { getByRole, container } = render(
      <Sparkline series={[45, 20, 90, 30, 60]} label="playtime, last 5 sessions" />,
    );
    const img = getByRole('img');
    expect(img.tagName.toLowerCase()).toBe('svg');
    expect(img).toHaveAttribute('aria-label', 'playtime, last 5 sessions');
    // A line path is always drawn; it must carry real move/line commands.
    const line = container.querySelector('path[stroke]');
    expect(line).not.toBeNull();
    expect(line?.getAttribute('d')).toMatch(/^M.*L/);
  });

  it('renders nothing for an empty series', () => {
    const { container } = render(<Sparkline series={[]} label="x" />);
    expect(container.querySelector('svg')).toBeNull();
  });

  it('renders a flat baseline (no crash) for a single-point series', () => {
    const { getByRole, container } = render(
      <Sparkline series={[42]} label="one point" />,
    );
    expect(getByRole('img')).toBeInTheDocument();
    // A single value still produces a drawable horizontal line.
    const line = container.querySelector('path[stroke]');
    expect(line?.getAttribute('d')).toMatch(/^M.*L/);
  });

  it('omits the area fill when area={false}', () => {
    const { container } = render(
      <Sparkline series={[1, 2, 3]} label="x" area={false} />,
    );
    // Only the stroked line path, no filled area path.
    const filled = container.querySelector('path[fill]:not([fill="none"])');
    expect(filled).toBeNull();
  });
});
