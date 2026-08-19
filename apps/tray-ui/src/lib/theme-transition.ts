import type { Theme } from '../api';
import { THEME_ACCENT } from './theme-accents';

export interface WaveOptions {
  /** Called exactly once to persist the choice (tray config draft). */
  onPersist?: (theme: Theme) => void;
  /** Override the sweep duration (ms). Defaults to 700. */
  durationMs?: number;
}

const WARP_CLASS = 'theme-warp';
const BEAM_CLASS = 'theme-wave-beam';
const DEFAULT_DURATION = 700;

/**
 * Wave-speed → duration map. Must stay in sync with the web version
 * (`apps/web/src/lib/theme-transition.ts`) and the server's
 * `ALLOWED_WAVE_SPEEDS` (`preferences_routes.rs` / `appearance_routes.rs`).
 * `off` is handled specially in `applyThemeWithWave` — it collapses into
 * the reduced-motion instant-swap path rather than running a 0ms beam.
 */
const WAVE_SPEED_MS: Readonly<Record<string, number>> = {
  off: 0,
  slow: 1100,
  normal: 700,
  fast: 350,
};

/**
 * Resolve the effective wave duration from `<html data-wave-speed>`,
 * stamped by `App.tsx` from `Config.theme_wave_speed` on boot and on
 * every config change (see `useEffect` there). Falls back to
 * `DEFAULT_DURATION` when the attribute is absent or unrecognised
 * (e.g. config hasn't loaded yet, or jsdom in tests) — matches the
 * pre-existing behaviour before wave speed shipped.
 */
function resolveWaveSpeedMs(): number {
  if (typeof document === 'undefined') return DEFAULT_DURATION;
  const raw = document.documentElement.dataset.waveSpeed;
  if (raw !== undefined && Object.prototype.hasOwnProperty.call(WAVE_SPEED_MS, raw)) {
    return WAVE_SPEED_MS[raw];
  }
  return DEFAULT_DURATION;
}

export function prefersReducedMotion(): boolean {
  return (
    typeof window !== 'undefined' &&
    typeof window.matchMedia === 'function' &&
    window.matchMedia('(prefers-reduced-motion: reduce)').matches
  );
}

export function supportsViewTransitions(): boolean {
  return (
    typeof document !== 'undefined' &&
    typeof (document as unknown as { startViewTransition?: unknown })
      .startViewTransition === 'function'
  );
}

let activeBeam: HTMLElement | null = null;
let generation = 0;

function removeBeam(): void {
  if (activeBeam) {
    activeBeam.remove();
    activeBeam = null;
  }
}

function runBeam(durationMs: number, leadAccent: string, trailAccent: string): void {
  const beam = document.createElement('div');
  beam.className = BEAM_CLASS;
  beam.style.setProperty('--beam-lead', leadAccent);
  beam.style.setProperty('--beam-trail', trailAccent);
  document.body.appendChild(beam);
  activeBeam = beam;

  // Element.animate is absent in jsdom; guard so tests don't throw.
  const anim =
    typeof beam.animate === 'function'
      ? beam.animate(
          [{ transform: 'translateY(-12vh)' }, { transform: 'translateY(112vh)' }],
          { duration: durationMs, easing: 'cubic-bezier(0.4, 0, 0.2, 1)' },
        )
      : null;

  const done = () => {
    beam.remove();
    if (activeBeam === beam) activeBeam = null;
  };
  if (anim) anim.finished.then(done, done);
}

/**
 * Apply `next` theme with an in-place top→bottom wave reveal.
 *
 * - unchanged theme → persist only, no animation
 * - reduced motion  → instant swap, no beam
 * - View Transitions → clip-path reveal (see theme-transition.css) + beam
 * - otherwise        → instant swap + beam
 *
 * Ported from `apps/web/src/lib/theme-transition.ts` — identical
 * branch logic, adapted to the tray's `Theme`/`THEME_ACCENT` sources.
 */
export function applyThemeWithWave(next: Theme, opts: WaveOptions = {}): void {
  if (typeof document === 'undefined') return;
  const root = document.documentElement;
  const current = (root.dataset.theme as Theme | undefined) ?? null;
  const persist = () => {
    const r = opts.onPersist?.(next) as unknown;
    if (r && typeof (r as { catch?: unknown }).catch === 'function') {
      (r as Promise<unknown>).catch(() => {});
    }
  };

  if (current === next) {
    persist();
    return;
  }

  const durationMs = opts.durationMs ?? resolveWaveSpeedMs();

  // Reduced motion OR wave speed 'off' (durationMs resolves to 0) both
  // want the same instant, beam-free swap.
  if (prefersReducedMotion() || durationMs <= 0) {
    root.dataset.theme = next;
    persist();
    return;
  }

  const leadAccent = THEME_ACCENT[next];
  const trailAccent = current ? THEME_ACCENT[current] : '#ffffff';

  removeBeam(); // cancel any in-flight beam (rapid re-clicks)

  if (supportsViewTransitions()) {
    const myGen = ++generation;
    root.classList.add(WARP_CLASS);
    const vt = (
      document as unknown as {
        startViewTransition: (cb: () => void) => { finished: Promise<void> };
      }
    ).startViewTransition(() => {
      root.dataset.theme = next;
    });
    runBeam(durationMs, leadAccent, trailAccent);
    const cleanup = () => {
      if (myGen === generation) root.classList.remove(WARP_CLASS);
    };
    vt.finished.then(cleanup, cleanup);
    persist();
    return;
  }

  root.dataset.theme = next;
  runBeam(durationMs, leadAccent, trailAccent);
  persist();
}
