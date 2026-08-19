import { describe, expect, it } from 'vitest';
import { THEMES } from '../api';
import { THEME_ACCENT } from './theme-accents';

describe('THEME_ACCENT', () => {
  it('has an accent hex for every theme', () => {
    for (const theme of THEMES) {
      expect(THEME_ACCENT[theme.id]).toMatch(/^#[0-9a-fA-F]{6}$/);
    }
  });

  it('has no extra keys beyond the known themes', () => {
    expect(Object.keys(THEME_ACCENT).sort()).toEqual(
      THEMES.map((t) => t.id).sort(),
    );
  });
});
