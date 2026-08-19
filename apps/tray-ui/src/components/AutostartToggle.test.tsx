import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { invoke } from '@tauri-apps/api/core';
import { AutostartToggle } from './AutostartToggle';

const mockedInvoke = vi.mocked(invoke);

describe('AutostartToggle', () => {
  beforeEach(() => {
    mockedInvoke.mockReset();
  });

  it('renders unchecked when get_autostart_enabled returns false', async () => {
    mockedInvoke.mockImplementation((cmd: string) => {
      if (cmd === 'get_autostart_enabled') return Promise.resolve(false);
      throw new Error(`unexpected command: ${cmd}`);
    });
    render(<AutostartToggle />);
    const checkbox = await screen.findByRole('checkbox', {
      name: /launch starstats at sign-in/i,
    });
    await waitFor(() => expect(checkbox).not.toBeDisabled());
    expect(checkbox).not.toBeChecked();
  });

  it('renders checked when get_autostart_enabled returns true', async () => {
    mockedInvoke.mockImplementation((cmd: string) => {
      if (cmd === 'get_autostart_enabled') return Promise.resolve(true);
      throw new Error(`unexpected command: ${cmd}`);
    });
    render(<AutostartToggle />);
    const checkbox = await screen.findByRole('checkbox', {
      name: /launch starstats at sign-in/i,
    });
    await waitFor(() => expect(checkbox).toBeChecked());
  });

  it('calls set_autostart_enabled with the new value on click', async () => {
    mockedInvoke.mockImplementation((cmd: string) => {
      if (cmd === 'get_autostart_enabled') return Promise.resolve(false);
      if (cmd === 'set_autostart_enabled') return Promise.resolve(undefined);
      throw new Error(`unexpected command: ${cmd}`);
    });
    const user = userEvent.setup();
    render(<AutostartToggle />);
    const checkbox = await screen.findByRole('checkbox', {
      name: /launch starstats at sign-in/i,
    });
    await waitFor(() => expect(checkbox).not.toBeDisabled());

    await user.click(checkbox);

    await waitFor(() => expect(checkbox).toBeChecked());
    expect(mockedInvoke).toHaveBeenCalledWith('set_autostart_enabled', {
      enabled: true,
    });
  });

  it('surfaces an error when set_autostart_enabled fails', async () => {
    mockedInvoke.mockImplementation((cmd: string) => {
      if (cmd === 'get_autostart_enabled') return Promise.resolve(false);
      if (cmd === 'set_autostart_enabled')
        return Promise.reject(new Error('write failed'));
      throw new Error(`unexpected command: ${cmd}`);
    });
    const user = userEvent.setup();
    render(<AutostartToggle />);
    const checkbox = await screen.findByRole('checkbox', {
      name: /launch starstats at sign-in/i,
    });
    await waitFor(() => expect(checkbox).not.toBeDisabled());

    await user.click(checkbox);

    await waitFor(() => expect(screen.getByText(/write failed/i)).toBeInTheDocument());
    // Toggle stays in the original (unchecked) state because the write
    // failed — we never optimistically update.
    expect(checkbox).not.toBeChecked();
  });
});
