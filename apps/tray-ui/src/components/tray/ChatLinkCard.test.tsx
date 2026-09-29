import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import { invoke } from '@tauri-apps/api/core';
import { ChatLinkCard } from './ChatLinkCard';

const mockedInvoke = vi.mocked(invoke);

function status(result: unknown, openResult: unknown = undefined) {
  mockedInvoke.mockImplementation(async (cmd: string) => {
    if (cmd === 'open_chat_window') {
      if (openResult instanceof Error) throw openResult;
      return openResult;
    }
    if (cmd !== 'chat_status') throw new Error(`unexpected ${cmd}`);
    if (result instanceof Error) throw result;
    return result;
  });
}

describe('ChatLinkCard', () => {
  beforeEach(() => {
    mockedInvoke.mockReset();
  });

  it('opens the chat window when the server offers chat', async () => {
    status({ offered: true });
    render(<ChatLinkCard webOrigin="https://starstats.app" />);
    fireEvent.click(await screen.findByRole('button', { name: 'Open chat' }));
    await waitFor(() => expect(mockedInvoke).toHaveBeenCalledWith('open_chat_window'));
    expect(screen.queryByRole('alert')).toBeNull();
  });

  it('says so when the window cannot open', async () => {
    status({ offered: true }, new Error('no web origin configured'));
    render(<ChatLinkCard webOrigin="https://starstats.app" />);
    fireEvent.click(await screen.findByRole('button', { name: 'Open chat' }));
    expect(await screen.findByRole('alert')).toHaveTextContent('no web origin configured');
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
