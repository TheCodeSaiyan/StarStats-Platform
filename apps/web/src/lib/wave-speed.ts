/**
 * Theme-switch wave animation speed. Mirrors `lib/theme.ts`'s
 * `Theme`/`isTheme` shape. Must stay in sync with the server's
 * `ALLOWED_WAVE_SPEEDS` (`preferences_routes.rs` / `appearance_routes.rs`)
 * and the client-side duration map in `theme-transition.ts`.
 */
export type WaveSpeed = 'off' | 'slow' | 'normal' | 'fast';

export const WAVE_SPEEDS: readonly WaveSpeed[] = [
  'off',
  'slow',
  'normal',
  'fast',
];
export const DEFAULT_WAVE_SPEED: WaveSpeed = 'normal';

const VALID = new Set<WaveSpeed>(WAVE_SPEEDS);

export function isWaveSpeed(value: unknown): value is WaveSpeed {
  return typeof value === 'string' && VALID.has(value as WaveSpeed);
}
