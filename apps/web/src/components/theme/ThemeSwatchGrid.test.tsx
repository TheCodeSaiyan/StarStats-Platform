import React from 'react';
import { describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen } from '@testing-library/react';

// vi.mock factories run before this file's own top-level statements
// (the mocked modules are transitively imported by ThemeSwatchGrid,
// which evaluates ahead of this file's body per ES module order), so
// the vi.fn()s referenced inside the factories must be created via
// vi.hoisted() — a plain top-level const here hits a TDZ crash.
const { applyThemeWithWave, persistThemeAction } = vi.hoisted(() => ({
  applyThemeWithWave: vi.fn(),
  persistThemeAction: vi.fn(),
}));
vi.mock('@/lib/theme-transition', () => ({ applyThemeWithWave }));
vi.mock('@/app/_actions/theme', () => ({ persistThemeAction }));

import { ThemeSwatchGrid } from '@/components/theme/ThemeSwatchGrid';

describe('ThemeSwatchGrid', () => {
  it('animates + persists the clicked theme instead of submitting', () => {
    const themeAction = vi.fn();
    render(<ThemeSwatchGrid activeTheme="stanton" themeAction={themeAction} />);

    fireEvent.click(screen.getByRole('button', { name: /Pyro/i }));

    expect(applyThemeWithWave).toHaveBeenCalledWith(
      'pyro',
      expect.objectContaining({ onPersist: persistThemeAction }),
    );
    expect(themeAction).not.toHaveBeenCalled();
  });

  it('marks the active swatch, moving it after a click', () => {
    render(<ThemeSwatchGrid activeTheme="stanton" themeAction={vi.fn()} />);
    expect(screen.getByRole('button', { name: /Stanton/i })).toHaveAttribute(
      'aria-pressed',
      'true',
    );
    fireEvent.click(screen.getByRole('button', { name: /Terra/i }));
    expect(screen.getByRole('button', { name: /Terra/i })).toHaveAttribute(
      'aria-pressed',
      'true',
    );
  });
});
