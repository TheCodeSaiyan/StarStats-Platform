import type { Theme } from '@/lib/theme';

/**
 * The signature accent colour per theme. Single source of truth for the
 * theme-transition beam tint and the Settings swatch accent chip. Values
 * mirror the accent stop of `:root[data-theme="…"]` in
 * `apps/web/src/styles/starstats-tokens.css`. `Record<Theme, string>` is
 * exhaustive by construction — a new Theme variant fails to compile here
 * until its accent is supplied.
 */
export const THEME_ACCENT: Record<Theme, string> = {
  stanton: '#E8A23C',
  pyro: '#F25C3F',
  terra: '#4FB8A1',
  nyx: '#5B3FD9',
};
