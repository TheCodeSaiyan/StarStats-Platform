/**
 * Vitest spec for the tray Social pane.
 *
 * Covers the rules that differ between right and wrong, not just rendering:
 *  - copy/add-in-game is disabled for an unverified handle, enabled otherwise
 *  - add in game copies the exact handle BEFORE opening RSI (focus rule)
 *  - accept sends the snake_case `request_id` key the Rust command expects
 *  - a server error code is shown as copy, not as the raw code
 *  - unpaired shows the pairing hint
 */

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import { invoke } from '@tauri-apps/api/core';
import { SocialPane, RSI_FRIENDS_URL, describeError } from './SocialPane';

const { openMock } = vi.hoisted(() => ({ openMock: vi.fn(async () => {}) }));
vi.mock('@tauri-apps/plugin-shell', () => ({ open: openMock }));
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async () => () => {}),
}));

const mockedInvoke = vi.mocked(invoke);

const FRIENDS = {
  friends: [
    { handle: 'Wingman', since: '2026-09-01T00:00:00Z', rsi_verified: true },
    { handle: 'Unproven', since: '2026-09-02T00:00:00Z', rsi_verified: false },
  ],
  incoming: [
    {
      id: '0199a000-0000-7000-8000-000000000001',
      requester_handle: 'Recruit',
      recipient_handle: 'Me',
      status: 'pending',
      created_at: '2026-09-25T00:00:00Z',
      responded_at: null,
    },
  ],
  outgoing: [],
  friend_request_policy: 'everyone',
};

let calls: Array<{ cmd: string; args: unknown }>;
let writeText: ReturnType<typeof vi.fn>;

function stub(overrides: Record<string, () => Promise<unknown>> = {}) {
  mockedInvoke.mockImplementation((cmd: string, args?: unknown) => {
    calls.push({ cmd, args });
    if (overrides[cmd]) return overrides[cmd]() as Promise<never>;
    switch (cmd) {
      case 'social_get_friends':
        return Promise.resolve(FRIENDS as never);
      case 'social_get_notifications':
        return Promise.resolve({ items: [], unread_count: 0 } as never);
      case 'social_get_blocks':
        return Promise.resolve({ blocks: [] } as never);
      case 'social_get_mutes':
        return Promise.resolve({ mutes: [] } as never);
      case 'social_get_prefs':
        return Promise.resolve({ toasts: true, quiet_in_game: true } as never);
      case 'social_respond':
        return Promise.resolve(undefined as never);
      default:
        return Promise.reject(new Error(`unexpected invoke: ${cmd}`));
    }
  });
}

beforeEach(() => {
  calls = [];
  mockedInvoke.mockReset();
  openMock.mockReset();
  openMock.mockImplementation(async () => {});
  writeText = vi.fn().mockResolvedValue(undefined);
  Object.defineProperty(navigator, 'clipboard', {
    value: { writeText },
    configurable: true,
  });
  stub();
});

function rowFor(handle: string): HTMLElement {
  const row = screen
    .getAllByTestId('friend-row')
    .find((r) => r.textContent?.includes(`@${handle}`));
  if (!row) throw new Error(`no row for ${handle}`);
  return row;
}

describe('SocialPane', () => {
  it('offers copy only for a verified handle', async () => {
    render(<SocialPane />);
    await screen.findByText('@Wingman');
    const verified = rowFor('Wingman').querySelectorAll('button');
    const unverified = rowFor('Unproven').querySelectorAll('button');
    expect(verified[0]).toHaveTextContent('Copy handle');
    expect(verified[0]).toBeEnabled();
    expect(unverified[0]).toBeDisabled();
    expect(unverified[0]).toHaveAttribute('title', 'Handle not verified with RSI');
    expect(unverified[1]).toBeDisabled();
  });

  it('add in game copies the handle before opening RSI friends', async () => {
    render(<SocialPane />);
    await screen.findByText('@Wingman');
    const addInGame = [...rowFor('Wingman').querySelectorAll('button')].find(
      (b) => b.textContent === 'Add in game',
    )!;
    fireEvent.click(addInGame);
    await waitFor(() => expect(openMock).toHaveBeenCalledWith(RSI_FRIENDS_URL));
    expect(writeText).toHaveBeenCalledWith('Wingman');
    expect(writeText.mock.invocationCallOrder[0]).toBeLessThan(
      openMock.mock.invocationCallOrder[0],
    );
  });

  it('accept sends the snake_case request_id key', async () => {
    render(<SocialPane />);
    fireEvent.click(await screen.findByRole('button', { name: 'Accept' }));
    await waitFor(() =>
      expect(calls.find((c) => c.cmd === 'social_respond')?.args).toEqual({
        request_id: '0199a000-0000-7000-8000-000000000001',
        action: 'accept',
      }),
    );
  });

  it('maps a server error code to copy', async () => {
    stub({
      social_send_request: () => Promise.reject('user_not_found'),
    });
    render(<SocialPane />);
    fireEvent.change(await screen.findByLabelText('StarStats handle'), {
      target: { value: 'ghost' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Send request' }));
    expect(
      await screen.findByText('No StarStats account exists for that handle.'),
    ).toBeInTheDocument();
  });

  it('explains pairing when the tray is not paired', () => {
    expect(describeError('tray is not paired (no api_url or token)')).toBe(
      'Pair this tray to use friends.',
    );
  });
});
