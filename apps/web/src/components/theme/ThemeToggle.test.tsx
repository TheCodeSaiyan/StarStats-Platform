import React from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen } from '@testing-library/react';

const { applyThemeWithWave, persistThemeAction } = vi.hoisted(() => ({
  applyThemeWithWave: vi.fn(),
  persistThemeAction: vi.fn(),
}));
vi.mock('@/lib/theme-transition', () => ({ applyThemeWithWave }));
vi.mock('@/app/_actions/theme', () => ({ persistThemeAction }));

import { ThemeToggle } from '@/components/theme/ThemeToggle';

afterEach(() => {
  document.documentElement.removeAttribute('data-theme');
  vi.clearAllMocks();
});

describe('ThemeToggle', () => {
  it('opens a 4-theme menu and applies the picked theme', () => {
    document.documentElement.dataset.theme = 'stanton';
    render(<ThemeToggle />);

    fireEvent.click(screen.getByRole('button', { name: /theme/i }));

    const items = screen.getAllByRole('menuitemradio');
    expect(items).toHaveLength(4);

    fireEvent.click(screen.getByRole('menuitemradio', { name: /Terra/i }));
    expect(applyThemeWithWave).toHaveBeenCalledWith(
      'terra',
      expect.objectContaining({ onPersist: persistThemeAction }),
    );
  });

  it('is collapsed by default', () => {
    render(<ThemeToggle />);
    expect(screen.queryByRole('menuitemradio')).toBeNull();
  });

  it('opens the menu on Space keydown (button keyboard contract)', () => {
    render(<ThemeToggle />);

    const trigger = screen.getByRole('button', { name: /change theme/i });
    fireEvent.keyDown(trigger, { key: ' ' });

    const items = screen.getAllByRole('menuitemradio');
    expect(items).toHaveLength(4);
  });

  it('closes the menu on Escape keydown', () => {
    render(<ThemeToggle />);

    fireEvent.click(screen.getByRole('button', { name: /theme/i }));
    expect(screen.getAllByRole('menuitemradio')).toHaveLength(4);

    fireEvent.keyDown(document, { key: 'Escape' });

    expect(screen.queryByRole('menuitemradio')).toBeNull();
  });
});
