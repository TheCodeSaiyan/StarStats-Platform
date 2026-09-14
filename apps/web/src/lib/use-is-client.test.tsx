import React from 'react';
import { describe, it, expect } from 'vitest';
import { renderToString } from 'react-dom/server';
import { render, screen } from '@testing-library/react';
import { useIsClient } from './use-is-client';

function Probe() {
  return <span data-testid="probe">{useIsClient() ? 'client' : 'server'}</span>;
}

describe('useIsClient', () => {
  // The server snapshot is what hydration compares against; if this ever
  // reported `true` on the server, every gated portal would mismatch.
  it('is false on the server render', () => {
    expect(renderToString(<Probe />)).toContain('server');
  });

  it('is true once rendered on the client', () => {
    render(<Probe />);
    expect(screen.getByTestId('probe').textContent).toBe('client');
  });
});
