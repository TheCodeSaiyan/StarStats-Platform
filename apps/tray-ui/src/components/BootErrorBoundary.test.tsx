import React from 'react';
import { describe, it, expect, vi, afterEach } from 'vitest';
import { render, screen } from '@testing-library/react';
import { BootErrorBoundary } from './BootErrorBoundary';

/**
 * The property under test is the one that matters operationally: a throw
 * during render must produce SOMETHING ON SCREEN rather than an empty
 * document. A blank tray window is indistinguishable from the webview
 * failing to start, and that ambiguity is what made the 0.1.14 blank-window
 * report so expensive to diagnose.
 *
 * So these assert on rendered text and on the container not being empty —
 * not on internal state — and the passing case is checked too, because a
 * boundary that swallows healthy renders would be worse than none.
 */
function Boom(): React.ReactElement {
  throw new Error('kaboom from render');
}

describe('BootErrorBoundary', () => {
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('renders children untouched when nothing throws', () => {
    const { container } = render(
      <BootErrorBoundary>
        <p>healthy content</p>
      </BootErrorBoundary>,
    );
    expect(screen.getByText('healthy content')).toBeTruthy();
    expect(container.textContent).not.toContain('stopped drawing');
  });

  it('shows a message instead of a blank window when a child throws', () => {
    // React logs the caught error; silence it so the suite output stays
    // readable, but keep the spy so the assertion below can prove we did
    // not simply swallow it.
    const spy = vi.spyOn(console, 'error').mockImplementation(() => {});

    const { container } = render(
      <BootErrorBoundary>
        <Boom />
      </BootErrorBoundary>,
    );

    // The load-bearing assertion: the document is NOT empty.
    expect(container.textContent?.trim()).not.toBe('');
    expect(screen.getByRole('alert')).toBeTruthy();
    expect(screen.getByText(/stopped drawing/i)).toBeTruthy();
    // The error text itself must survive to the screen — it is the part a
    // user pastes into a bug report.
    expect(screen.getByText(/kaboom from render/)).toBeTruthy();
    expect(spy).toHaveBeenCalled();
  });

  it('offers a way back rather than a dead end', () => {
    vi.spyOn(console, 'error').mockImplementation(() => {});
    render(
      <BootErrorBoundary>
        <Boom />
      </BootErrorBoundary>,
    );
    expect(screen.getByRole('button', { name: /reload/i })).toBeTruthy();
  });
});
