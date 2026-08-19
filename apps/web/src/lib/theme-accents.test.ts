import { describe, expect, it } from 'vitest';
import { THEMES } from '@/lib/theme';
import { THEME_ACCENT } from '@/lib/theme-accents';

describe('THEME_ACCENT', () => {
  it('has an accent hex for every theme', () => {
    for (const theme of THEMES) {
      expect(THEME_ACCENT[theme]).toMatch(/^#[0-9a-fA-F]{6}$/);
    }
  });

  it('has no extra keys beyond the known themes', () => {
    expect(Object.keys(THEME_ACCENT).sort()).toEqual([...THEMES].sort());
  });
});
