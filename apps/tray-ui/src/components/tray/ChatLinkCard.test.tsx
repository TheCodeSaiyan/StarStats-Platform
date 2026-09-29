import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import { invoke } from '@tauri-apps/api/core';
import { ChatLinkCard } from './ChatLinkCard';

const { openMock } = vi.hoisted(() => ({
  openMock: vi.fn(async () => {}),
}));
vi.mock('@tauri-apps/plugin-shell', () => ({
  open: openMock,
}));

const mockedInvoke = vi.mocked(invoke);

function status(result: unknown) {
  mockedInvoke.mockImplementation(async (cmd: string) => {
    if (cmd !== 'chat_status') throw new Error(`unexpected ${cmd}`);
    if (result instanceof Error) throw result;
    return result;
  });
}

describe('ChatLinkCard', () => {
  beforeEach(() => {
    mockedInvoke.mockReset();
    openMock.mockClear();
  });

  it('opens web chat when the server offers chat', async () => {
    status({ offered: true });
    render(<ChatLinkCard webOrigin="https://starstats.app" />);
    fireEvent.click(await screen.findByRole('button', { name: 'Open chat' }));
    expect(openMock).toHaveBeenCalledWith('https://starstats.app/chat');
  });

  it.each([
    ['not offered', { offered: false }],
    ['an older server without the field', { available: true }],
    ['a failed call', new Error('offline')],
  ])('shows nothing for %s', async (_name, result) => {
    status(result);
    const { container } = render(<ChatLinkCard webOrigin="https://starstats.app" />);
    await waitFor(() => expect(mockedInvoke).toHaveBeenCalledWith('chat_status'));
    await Promise.resolve();
    expect(container).toBeEmptyDOMElement();
  });

  it('shows nothing without a web origin to open', async () => {
    status({ offered: true });
    const { container } = render(<ChatLinkCard webOrigin={null} />);
    await waitFor(() => expect(mockedInvoke).toHaveBeenCalledWith('chat_status'));
    await Promise.resolve();
    expect(container).toBeEmptyDOMElement();
  });
});
