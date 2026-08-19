import React from 'react';
import { describe, it, expect } from 'vitest';
import { render } from '@testing-library/react';
import Loading from './loading';

describe('admin/loading', () => {
  it('renders a busy skeleton fallback', () => {
    const { container } = render(<Loading />);
    // The Suspense fallback must announce itself to AT so the cold
    // admin→admin navigation isn't a silent freeze.
    expect(container.querySelector('[aria-busy="true"]')).not.toBeNull();
    expect(container.querySelectorAll('.skeleton').length).toBeGreaterThan(0);
  });

  it('does NOT render a <main> element (admin layout owns the sole landmark)', () => {
    // admin/layout.tsx wraps the surface in role="main"; a nested
    // <main> here would duplicate the landmark AND inherit the global
    // `main {}` 720px clamp, crushing the full-width admin skeleton.
    const { container } = render(<Loading />);
    expect(container.querySelector('main')).toBeNull();
  });
});
