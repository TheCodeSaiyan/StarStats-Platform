import { describe, it, expect, vi } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { InlineCheck } from './InlineCheck';

describe('InlineCheck', () => {
  it('is disabled when input is empty', () => {
    render(<InlineCheck label="Test" value="" onCheck={vi.fn()} />);
    expect(screen.getByRole('button', { name: /test/i })).toBeDisabled();
  });

  it('shows a success line after a successful check', async () => {
    const user = userEvent.setup();
    const onCheck = vi.fn().mockResolvedValue({ ok: true, message: 'reachable' });
    render(<InlineCheck label="Test" value="https://x" onCheck={onCheck} />);
    await user.click(screen.getByRole('button', { name: /test/i }));
    await waitFor(() => expect(screen.getByText(/reachable/i)).toBeInTheDocument());
  });

  it('shows an error line after a failed check', async () => {
    const user = userEvent.setup();
    const onCheck = vi.fn().mockResolvedValue({ ok: false, message: 'HTTP 404' });
    render(<InlineCheck label="Test" value="https://x" onCheck={onCheck} />);
    await user.click(screen.getByRole('button', { name: /test/i }));
    await waitFor(() => expect(screen.getByText(/HTTP 404/)).toBeInTheDocument());
  });

  it('disables the button while running', async () => {
    const user = userEvent.setup();
    const onCheck = vi.fn(() => new Promise<{ ok: boolean; message: string }>(() => {})); // never resolves
    render(<InlineCheck label="Test" value="https://x" onCheck={onCheck} />);
    await user.click(screen.getByRole('button', { name: /test/i }));
    expect(screen.getByRole('button', { name: /testing|test/i })).toBeDisabled();
  });
});
