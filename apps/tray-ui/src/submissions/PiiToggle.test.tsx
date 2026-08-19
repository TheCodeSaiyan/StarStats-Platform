import { describe, it, expect, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { PiiToggle, type PiiToken } from './PiiToggle';

const ownHandle: PiiToken = {
  kind: 'own_handle',
  start: 0,
  end: 4,
  suggested_redaction: '[HANDLE]',
  default_redact: true,
};

describe('PiiToggle', () => {
  it('renders with default redact state from the token', () => {
    render(<PiiToggle token={ownHandle} onChange={() => {}} />);
    const checkbox = screen.getByRole('checkbox') as HTMLInputElement;
    expect(checkbox.checked).toBe(true);
  });

  it('calls onChange when flipped', async () => {
    const onChange = vi.fn();
    render(<PiiToggle token={ownHandle} onChange={onChange} />);
    await userEvent.click(screen.getByRole('checkbox'));
    expect(onChange).toHaveBeenCalledWith(false);
  });

  it('shows the suggested redaction text', () => {
    render(<PiiToggle token={ownHandle} onChange={() => {}} />);
    expect(screen.getByText(/\[HANDLE\]/)).toBeInTheDocument();
  });

  it('starts unchecked when default_redact is false', () => {
    const token: PiiToken = { ...ownHandle, kind: 'friend_handle', default_redact: false };
    render(<PiiToggle token={token} onChange={() => {}} />);
    const checkbox = screen.getByRole('checkbox') as HTMLInputElement;
    expect(checkbox.checked).toBe(false);
  });
});
