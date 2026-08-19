/**
 * Theme-switch wave animation speed. Mirrors the web `lib/wave-speed.ts`
 * shape and must stay in sync with the Rust `theme_wave_speed` field
 * (`crates/starstats-client/src/config.rs`) and the server's
 * `ALLOWED_WAVE_SPEEDS` (`preferences_routes.rs` / `appearance_routes.rs`).
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
