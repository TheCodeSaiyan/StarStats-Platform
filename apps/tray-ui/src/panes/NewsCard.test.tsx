import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import { invoke } from '@tauri-apps/api/core';
import { NewsCard } from './NewsCard';

const { openMock } = vi.hoisted(() => ({ openMock: vi.fn(async () => {}) }));
vi.mock('@tauri-apps/plugin-shell', () => ({ open: openMock }));

const mockedInvoke = vi.mocked(invoke);

const item = {
  id: '0199b000-0000-7000-8000-000000000001',
  title: 'Scheduled maintenance',
  body: 'Down 02:00-03:00 UTC.\n<b>not bold</b>',
  link_url: 'https://starstats.app/status',
  published_at: '2026-09-26T10:00:00Z',
  unread: true,
};

let calls: Array<{ cmd: string; args: unknown }>;

beforeEach(() => {
  calls = [];
  mockedInvoke.mockReset();
  openMock.mockReset();
  mockedInvoke.mockImplementation(async (cmd: string, args?: unknown) => {
    calls.push({ cmd, args });
    if (cmd === 'get_news') return { items: [item], unread_count: 1 };
    if (cmd === 'mark_news_seen') return undefined;
    throw new Error(`unexpected ${cmd}`);
  });
});

describe('NewsCard', () => {
  it('opening a post shows its text literally and marks it read', async () => {
    const { container } = render(<NewsCard />);
    fireEvent.click(await screen.findByRole('button', { name: /Scheduled maintenance/ }));
    expect(screen.getByText(/<b>not bold<\/b>/)).toBeInTheDocument();
    expect(container.querySelector('b')).toBeNull();
    await waitFor(() =>
      expect(calls.find((c) => c.cmd === 'mark_news_seen')?.args).toEqual({
        news_id: item.id,
      }),
    );
    fireEvent.click(screen.getByRole('button', { name: 'Read more →' }));
    expect(openMock).toHaveBeenCalledWith('https://starstats.app/status');
  });

  it('renders nothing when unpaired', async () => {
    mockedInvoke.mockImplementation(async () => {
      throw new Error('tray is not paired');
    });
    const { container } = render(<NewsCard />);
    await waitFor(() => expect(mockedInvoke).toHaveBeenCalled());
    expect(container).toBeEmptyDOMElement();
  });
});
