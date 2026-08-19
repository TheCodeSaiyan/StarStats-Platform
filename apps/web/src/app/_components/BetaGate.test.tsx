import React from 'react';
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';

// The form calls a server action; stub it so the overlay renders in jsdom.
// Deferred-call shape (not an inline vi.fn) so the attribution tests can
// assert what FormData the gate actually submitted — same pattern as
// WaitlistForm.test.tsx.
const action = vi.fn(async (_fd: FormData) => ({ ok: true, position: 7 }));
vi.mock('../_actions/waitlist', () => ({
  joinWaitlistAction: (...a: unknown[]) => action(...(a as [FormData])),
}));

import { BetaGate } from './BetaGate';

describe('BetaGate', () => {
  beforeEach(() => {
    document.cookie = 'ss_beta_dismissed=; Max-Age=0; path=/';
  });

  it('renders the waitlist form inside a modal dialog', () => {
    render(<BetaGate />);
    expect(screen.getByRole('dialog')).toHaveAttribute('aria-modal', 'true');
    expect(
      screen.getByRole('button', { name: /join the waitlist/i })
    ).toBeInTheDocument();
  });

  it('dismiss hides the overlay and sets the cookie', () => {
    render(<BetaGate />);
    fireEvent.click(screen.getByRole('button', { name: /browse the site/i }));
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    expect(document.cookie).toContain('ss_beta_dismissed=1');
  });

  it('Escape dismisses the overlay', () => {
    render(<BetaGate />);
    fireEvent.keyDown(document, { key: 'Escape' });
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  });
});

describe('BetaGate channel attribution', () => {
  afterEach(() => {
    action.mockClear();
    window.sessionStorage.clear();
    window.history.replaceState({}, '', '/');
  });

  const submit = () => {
    fireEvent.change(screen.getByLabelText(/email/i), {
      target: { value: 'a@example.com' },
    });
    fireEvent.click(screen.getByRole('button', { name: /join the waitlist/i }));
  };

  const submittedSource = () => {
    const fd = action.mock.calls[0][0] as FormData;
    return fd.get('source');
  };

  it('attributes the signup to the ?src= param when present', async () => {
    window.history.replaceState({}, '', '/?src=reddit');
    render(<BetaGate />);
    submit();
    await waitFor(() => expect(action).toHaveBeenCalled());
    expect(submittedSource()).toBe('reddit');
  });

  it('stores the param so a later join still attributes', async () => {
    window.history.replaceState({}, '', '/?src=reddit');
    render(<BetaGate />);
    await waitFor(() =>
      expect(window.sessionStorage.getItem('ss_src')).toBe('reddit'),
    );
  });

  it('remembers the channel across a navigation that loses the param', async () => {
    // A prior visit stored it; this render has a bare URL.
    window.sessionStorage.setItem('ss_src', 'spectrum-org');
    render(<BetaGate />);
    submit();
    await waitFor(() => expect(action).toHaveBeenCalled());
    expect(submittedSource()).toBe('spectrum-org');
  });

  it('falls back to beta-gate with no param and nothing stored', async () => {
    render(<BetaGate />);
    submit();
    await waitFor(() => expect(action).toHaveBeenCalled());
    expect(submittedSource()).toBe('beta-gate');
  });
});
