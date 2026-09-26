import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import { invoke } from '@tauri-apps/api/core';
import { ReleasesCard } from './ReleasesCard';

const mockedInvoke = vi.mocked(invoke);

const release = (over: Record<string, unknown>) => ({
  id: 'r1',
  track: 'tray',
  tag: 'tray-v0.1.31',
  version: '0.1.31',
  channel: 'live',
  released_on: '2026-09-26',
  summary: '1 new, 1 fixed',
  notes: [
    { kind: 'New', lines: [{ text: 'Friends & notifications: Add friends <b>now</b>' }] },
    { kind: 'Fixed', lines: [{ text: 'Group the Review tab by line type' }] },
  ],
  unread: true,
  ...over,
});

let calls: Array<{ cmd: string; args: unknown }>;

beforeEach(() => {
  calls = [];
  mockedInvoke.mockReset();
  mockedInvoke.mockImplementation(async (cmd: string, args?: unknown) => {
    calls.push({ cmd, args });
    if (cmd === 'get_releases') {
      return {
        releases: [
          release({}),
          release({ id: 'r0', tag: 'tray-v0.1.30', version: '0.1.30', summary: '1 fixed', unread: true }),
        ],
        unread_count: 2,
      };
    }
    if (cmd === 'mark_release_seen') return undefined;
    throw new Error(`unexpected ${cmd}`);
  });
});

describe('ReleasesCard', () => {
  it('opens the newest release, marks only it read, and folds the rest', async () => {
    const { container } = render(<ReleasesCard />);
    expect(await screen.findByText('Group the Review tab by line type')).toBeInTheDocument();
    await waitFor(() =>
      expect(calls.filter((c) => c.cmd === 'mark_release_seen').map((c) => c.args)).toEqual([
        { release_id: 'r1' },
      ]),
    );
    // Markup in a note arrives as text.
    expect(screen.getByText(/<b>now<\/b>/)).toBeInTheDocument();
    expect(container.querySelector('b')).toBeNull();
    // The older release is folded until opened.
    const older = screen.getByRole('button', { name: /StarStats 0\.1\.30/ });
    expect(older).toHaveAttribute('aria-expanded', 'false');
    fireEvent.click(older);
    await waitFor(() =>
      expect(calls.filter((c) => c.cmd === 'mark_release_seen')).toHaveLength(2),
    );
  });

  it('renders nothing when unpaired', async () => {
    mockedInvoke.mockImplementation(async () => {
      throw new Error('tray is not paired');
    });
    const { container } = render(<ReleasesCard />);
    await waitFor(() => expect(mockedInvoke).toHaveBeenCalled());
    expect(container).toBeEmptyDOMElement();
  });
});
