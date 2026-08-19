// Explicit React import: this repo's vitest uses the classic JSX
// runtime, so a JSX-rendering component ReferenceErrors without it.
import React from 'react';
import { describe, expect, it } from 'vitest';
import { render, screen } from '@testing-library/react';
import { AdminPageHeader } from './AdminPageHeader';

describe('AdminPageHeader', () => {
  it('renders the title as the page h1', () => {
    render(<AdminPageHeader eyebrow="Admin · users" title="Users" />);
    expect(
      screen.getByRole('heading', { level: 1, name: 'Users' }),
    ).toBeInTheDocument();
  });

  it('renders the eyebrow and lede', () => {
    render(
      <AdminPageHeader eyebrow="Admin · users" title="Users" lede="Search." />,
    );
    expect(screen.getByText('Admin · users')).toBeInTheDocument();
    expect(screen.getByText('Search.')).toBeInTheDocument();
  });

  it('omits the lede paragraph when not given', () => {
    const { container } = render(
      <AdminPageHeader eyebrow="Admin" title="Users" />,
    );
    expect(container.querySelectorAll('p')).toHaveLength(0);
  });

  // The lede is ReactNode, not string: ship-matrix and smtp both pass
  // markup (<strong>, entities). Typing it as string would have forced
  // those ledes to be flattened and lost their emphasis.
  it('accepts markup in the lede', () => {
    render(
      <AdminPageHeader
        eyebrow="Admin · Ship Matrix"
        title="Ship Matrix"
        lede={
          <>
            This toggle controls the <strong>images</strong>.
          </>
        }
      />,
    );
    expect(screen.getByText('images').tagName).toBe('STRONG');
  });
});
