import { render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { UpdateBanner } from './UpdateBanner';
import { checkForUpdate } from '../updater';

vi.mock('../updater', () => ({
  checkForUpdate: vi.fn(),
  applyUpdate: vi.fn(),
}));

describe('UpdateBanner', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('shows an install banner when an update is available', async () => {
    vi.mocked(checkForUpdate).mockResolvedValue({
      available: true,
      version: '1.8.47',
      notes: null,
      date: null,
    });
    render(<UpdateBanner channel="live" autoCheck />);
    await waitFor(() =>
      expect(screen.getByText(/Update available/)).toBeInTheDocument(),
    );
    expect(screen.getByText(/v1\.8\.47/)).toBeInTheDocument();
    expect(
      screen.getByRole('button', { name: /install/i }),
    ).toBeInTheDocument();
  });

  it('shows a manual check button and "up to date" when no update', async () => {
    vi.mocked(checkForUpdate).mockResolvedValue({ available: false });
    render(<UpdateBanner channel="live" autoCheck />);
    await waitFor(() =>
      expect(screen.getByText(/up to date/i)).toBeInTheDocument(),
    );
    expect(
      screen.getByRole('button', { name: /check for updates/i }),
    ).toBeInTheDocument();
  });

  it('does NOT auto-check on mount when auto-check is disabled', async () => {
    render(<UpdateBanner channel="live" autoCheck={false} />);
    // The manual button is available, but no unsolicited check fired.
    expect(
      screen.getByRole('button', { name: /check for updates/i }),
    ).toBeInTheDocument();
    expect(screen.queryByText(/up to date/i)).not.toBeInTheDocument();
    expect(checkForUpdate).not.toHaveBeenCalled();
  });
});
