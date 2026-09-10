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
    // A private window / blocked site data throws on ACCESS.
    //
    // The whole `localStorage` property is REPLACED rather than spied on with
    // `vi.spyOn(window.localStorage, 'getItem')`. jsdom implements Storage
    // behind a Proxy so that `localStorage.foo = 1` writes a stored item, and
    // that Proxy's `defineProperty` trap swallows the own-property vitest
    // installs to shadow the prototype method — so the spy is never the
    // function the component calls, and `toHaveBeenCalled()` fails while the
    // component works fine.
    //
    // It failed on CI (Node 24) and passed on Node 26, where a native
    // `localStorage` shadows the jsdom one and takes the spy normally. The
    // version skew is what kept it hidden; replacing the property is correct
    // under both. `readDismissed` reads `window.localStorage` at call time, so
    // the stub is what it gets.
    const get = vi.fn(() => {
      throw new Error('blocked');
    });
    const set = vi.fn(() => {
      throw new Error('blocked');
    });
    const original = Object.getOwnPropertyDescriptor(window, 'localStorage');
    Object.defineProperty(window, 'localStorage', {
      configurable: true,
      value: { getItem: get, setItem: set },
    });

    try {
      const user = userEvent.setup();
      render(<EmitterPrompt />);
      expect(await screen.findByRole('dialog')).toBeInTheDocument();
      expect(get).toHaveBeenCalled();
      // Dismissal must not blow up just because it cannot be remembered.
      await user.click(
        screen.getByRole('button', { name: /look around first/i }),
      );
      expect(screen.queryByRole('dialog')).toBeNull();
      expect(set).toHaveBeenCalled();
    } finally {
      // Restore in `finally`: leaking the throwing stub would break the
      // `beforeEach` clear() of every test that runs after this one in
      // the same file.
      //
      // `localStorage` may be an own accessor or inherited from the window
      // prototype depending on the runtime, so put back what was actually
      // there — and when there was no own property, delete ours so the
      // inherited one resurfaces rather than staying shadowed.
      if (original) {
        Object.defineProperty(window, 'localStorage', original);
      } else {
        delete (window as { localStorage?: unknown }).localStorage;
      }
    }
  });
});
