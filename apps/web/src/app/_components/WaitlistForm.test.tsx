import React from 'react';
import { describe, it, expect, vi, afterEach } from 'vitest';
import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from '@testing-library/react';

const action = vi.fn();
vi.mock('@/app/_actions/waitlist', () => ({
  joinWaitlistAction: (...a: unknown[]) => action(...a),
}));

import { WaitlistForm } from './WaitlistForm';

afterEach(() => {
  cleanup();
  action.mockReset();
});

const submit = (email = 'a@example.com') => {
  fireEvent.change(screen.getByLabelText(/email/i), { target: { value: email } });
  fireEvent.click(screen.getByRole('button', { name: /join/i }));
};

describe('WaitlistForm', () => {
  it('shows the queue position when queued', async () => {
    action.mockImplementation(async () => ({ ok: true, position: 12 }));
    render(<WaitlistForm />);
    submit();
    await waitFor(() => expect(screen.getByText(/12/)).toBeTruthy());
  });

  it('tells an admitted user to check their email', async () => {
    action.mockImplementation(async () => ({ ok: true, position: null }));
    render(<WaitlistForm />);
    submit();
    await waitFor(() =>
      expect(screen.getByText(/check your email/i)).toBeTruthy(),
    );
  });

  it('shows an error instead of a false success', async () => {
    action.mockImplementation(async () => ({ ok: false, error: 'join_failed' }));
    render(<WaitlistForm />);
    submit();
    await waitFor(() => expect(screen.getByRole('alert')).toBeTruthy());
    // A failure must never render the success copy — the whole point of
    // the discriminated union.
    expect(screen.queryByText(/check your email/i)).toBeNull();
  });

  it('names a bad email specifically rather than blaming the server', async () => {
    action.mockImplementation(async () => ({
      ok: false,
      error: 'invalid_email',
    }));
    render(<WaitlistForm />);
    submit('nope');
    await waitFor(() => expect(screen.getByRole('alert')).toBeTruthy());
    expect(screen.getByRole('alert').textContent).toMatch(/email/i);
  });

  it('passes the source through when given one', async () => {
    action.mockImplementation(async () => ({ ok: true, position: 1 }));
    render(<WaitlistForm source="reddit" />);
    submit();
    await waitFor(() => expect(action).toHaveBeenCalled());
    const fd = action.mock.calls[0][0] as FormData;
    expect(fd.get('source')).toBe('reddit');
    expect(fd.get('email')).toBe('a@example.com');
  });

  it('disables the button while in flight so one click is one signup', async () => {
    let release: (v: unknown) => void = () => {};
    action.mockImplementation(
      () => new Promise((r) => { release = r; }),
    );
    render(<WaitlistForm />);
    submit();
    await waitFor(() =>
      expect(screen.getByRole('button', { name: /join/i })).toHaveProperty(
        'disabled',
        true,
      ),
    );
    release({ ok: true, position: 1 });
  });

  it('replaces the form with the result rather than inviting a resubmit', async () => {
    action.mockImplementation(async () => ({ ok: true, position: 3 }));
    render(<WaitlistForm />);
    submit();
    await waitFor(() => expect(screen.getByText(/3/)).toBeTruthy());
    expect(screen.queryByRole('button', { name: /join/i })).toBeNull();
  });

  it('uses unique email input IDs when multiple forms are mounted', () => {
    render(
      <>
        <WaitlistForm />
        <WaitlistForm />
      </>,
    );
    const ids = screen.getAllByLabelText('Waitlist email').map((input) => input.id);
    expect(new Set(ids).size).toBe(2);
  });
});
