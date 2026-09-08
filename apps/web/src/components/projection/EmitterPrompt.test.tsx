import React from 'react';
import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { EmitterPrompt } from './EmitterPrompt';

vi.mock('next/link', () => ({
  default: ({
    href,
    children,
    ...rest
  }: {
    href: string;
    children: React.ReactNode;
  }) => (
    <a href={href} {...rest}>
      {children}
    </a>
  ),
}));

const KEY = 'starstats.emitter-prompt.dismissed';

describe('EmitterPrompt', () => {
  beforeEach(() => {
    window.localStorage.clear();
  });

  it('tells a first-run reader to install the Emitter and points at the download', async () => {
    render(<EmitterPrompt />);
    const dialog = await screen.findByRole('dialog');
    expect(dialog).toHaveTextContent(/empty until you add an emitter/i);
    expect(screen.getByRole('link', { name: /get the emitter/i })).toHaveAttribute(
      'href',
      '/downloads',
    );
  });

  it('stays dismissed once the reader has dismissed it', async () => {
    const user = userEvent.setup();
    const { unmount } = render(<EmitterPrompt />);
    await screen.findByRole('dialog');
    await user.click(screen.getByRole('button', { name: /look around first/i }));
    expect(screen.queryByRole('dialog')).toBeNull();
    expect(window.localStorage.getItem(KEY)).toBe('1');

    // A later visit must not show it again.
    unmount();
    render(<EmitterPrompt />);
    expect(screen.queryByRole('dialog')).toBeNull();
  });

  it('dismisses on Escape', async () => {
    const user = userEvent.setup();
    render(<EmitterPrompt />);
    await screen.findByRole('dialog');
    await user.keyboard('{Escape}');
    expect(screen.queryByRole('dialog')).toBeNull();
    expect(window.localStorage.getItem(KEY)).toBe('1');
  });

  it('still shows when localStorage throws, rather than staying hidden', async () => {
    // A private window / blocked site data throws on ACCESS. Spied on the
    // instance, not `Storage.prototype`: the value under test has to be the one
    // the component actually reads.
    const get = vi
      .spyOn(window.localStorage, 'getItem')
      .mockImplementation(() => {
        throw new Error('blocked');
      });
    const set = vi
      .spyOn(window.localStorage, 'setItem')
      .mockImplementation(() => {
        throw new Error('blocked');
      });
    const user = userEvent.setup();
    render(<EmitterPrompt />);
    expect(await screen.findByRole('dialog')).toBeInTheDocument();
    expect(get).toHaveBeenCalled();
    // Dismissal must not blow up just because it cannot be remembered.
    await user.click(screen.getByRole('button', { name: /look around first/i }));
    expect(screen.queryByRole('dialog')).toBeNull();
    expect(set).toHaveBeenCalled();
    get.mockRestore();
    set.mockRestore();
  });
});
