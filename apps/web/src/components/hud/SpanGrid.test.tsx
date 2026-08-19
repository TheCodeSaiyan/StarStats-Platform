import React from 'react';
import { describe, it, expect } from 'vitest';
import { render } from '@testing-library/react';
import { SpanGrid } from './SpanGrid';
describe('SpanGrid', () => {
  it('wraps children in a grid container', () => {
    const { container } = render(<SpanGrid><div data-testid="c">x</div></SpanGrid>);
    expect((container.firstChild as HTMLElement).className).toContain('hud-grid');
    expect(container.querySelector('[data-testid="c"]')).toBeInTheDocument();
  });
});
