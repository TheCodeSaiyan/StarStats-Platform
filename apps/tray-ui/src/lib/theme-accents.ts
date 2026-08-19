import { THEMES, type Theme } from '../api';

/**
 * The signature accent colour per theme. Single source of truth for the
 * theme-transition beam tint. Values are pulled from `THEMES` in `../api`
 * (`swatch.accent`) — that array is itself the tray's mirror of
 * `starstats-tokens.css`, so this stays in sync automatically rather
 * than duplicating a second copy of the hex values (as the web
 * `theme-accents.ts` does against its own `THEMES`). Derived at
 * runtime from `THEMES`, so — unlike the web version's object literal —
 * a new `Theme` variant added to `THEMES` picks up an accent
 * automatically instead of failing to compile.
 */
export const THEME_ACCENT: Record<Theme, string> = THEMES.reduce(
  (acc, t) => {
    acc[t.id] = t.swatch.accent;
    return acc;
  },
  {} as Record<Theme, string>,
);
