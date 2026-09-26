import React from 'react';
import { describe, it, expect, vi, afterEach, beforeEach } from 'vitest';
import { render, screen, cleanup, fireEvent, waitFor } from '@testing-library/react';

import { CopyHandleButton, RSI_FRIENDS_URL } from './CopyHandleButton';

let writeText: ReturnType<typeof vi.fn>;
let openSpy: ReturnType<typeof vi.spyOn>;

beforeEach(() => {
  writeText = vi.fn().mockResolvedValue(undefined);
  Object.defineProperty(navigator, 'clipboard', {
    value: { writeText },
    configurable: true,
  });
  openSpy = vi.spyOn(window, 'open').mockReturnValue(null);
});

afterEach(() => {
  cleanup();
  openSpy.mockRestore();
});

describe('CopyHandleButton', () => {
  it('copies exactly the verified handle and announces it', async () => {
    render(<CopyHandleButton handle="TheCodeSaiyan" verified />);
    fireEvent.click(screen.getByRole('button', { name: 'Copy handle' }));
    expect(writeText).toHaveBeenCalledWith('TheCodeSaiyan');
    expect(openSpy).not.toHaveBeenCalled();
    await waitFor(() =>
      expect(screen.getByRole('status')).toHaveTextContent('Handle copied'),
    );
  });

  it('is disabled for an unverified handle and copies nothing', () => {
    render(<CopyHandleButton handle="SomeoneElse" verified={false} addInGame />);
    const button = screen.getByRole('button', { name: 'Add in game' });
    expect(button).toBeDisabled();
    expect(button).toHaveAttribute('title', 'Handle not verified with RSI');
    fireEvent.click(button);
    expect(writeText).not.toHaveBeenCalled();
    expect(openSpy).not.toHaveBeenCalled();
  });

  it('add in game copies first, then opens RSI friend management', async () => {
    render(<CopyHandleButton handle="Wingman_7" verified addInGame />);
    fireEvent.click(screen.getByRole('button', { name: 'Add in game' }));
    expect(writeText).toHaveBeenCalledWith('Wingman_7');
    expect(openSpy).toHaveBeenCalledWith(
      RSI_FRIENDS_URL,
      '_blank',
      'noopener,noreferrer',
    );
    // Copy must be started before the tab steals focus.
    expect(writeText.mock.invocationCallOrder[0]).toBeLessThan(
      openSpy.mock.invocationCallOrder[0],
    );
    await waitFor(() =>
      expect(screen.getByRole('status')).toHaveTextContent(
        "paste it into RSI's add-friend box",
      ),
    );
  });

  it('says so when the clipboard refuses', async () => {
    writeText.mockRejectedValueOnce(new Error('denied'));
    render(<CopyHandleButton handle="Wingman_7" verified />);
    fireEvent.click(screen.getByRole('button', { name: 'Copy handle' }));
    await waitFor(() =>
      expect(screen.getByRole('status')).toHaveTextContent('Copy failed'),
    );
  });
});
