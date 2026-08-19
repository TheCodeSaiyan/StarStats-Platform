import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { applyThemeWithWave } from './theme-transition';

function setReducedMotion(reduce: boolean) {
  window.matchMedia = vi.fn().mockImplementation((q: string) => ({
    matches: q.includes('reduce') ? reduce : false,
    media: q,
    addEventListener: vi.fn(),
    removeEventListener: vi.fn(),
    addListener: vi.fn(),
    removeListener: vi.fn(),
    dispatchEvent: vi.fn(),
    onchange: null,
  })) as unknown as typeof window.matchMedia;
}

afterEach(() => {
  document.documentElement.className = '';
  document.documentElement.removeAttribute('data-theme');
  document.documentElement.removeAttribute('data-wave-speed');
  document.querySelectorAll('.theme-wave-beam').forEach((n) => n.remove());
  // @ts-expect-error reset the mock between tests
  delete document.startViewTransition;
  vi.restoreAllMocks();
});

describe('applyThemeWithWave', () => {
  beforeEach(() => {
    document.documentElement.dataset.theme = 'stanton';
    setReducedMotion(false);
  });

  it('no-ops the visual swap but still persists when theme is unchanged', () => {
    const onPersist = vi.fn();
    applyThemeWithWave('stanton', { onPersist });
    expect(onPersist).toHaveBeenCalledWith('stanton');
    expect(document.querySelector('.theme-wave-beam')).toBeNull();
  });

  it('under reduced motion swaps instantly with no beam', () => {
    setReducedMotion(true);
    const onPersist = vi.fn();
    applyThemeWithWave('pyro', { onPersist });
    expect(document.documentElement.dataset.theme).toBe('pyro');
    expect(document.querySelector('.theme-wave-beam')).toBeNull();
    expect(onPersist).toHaveBeenCalledWith('pyro');
  });

  it('without View Transitions, swaps instantly and appends a beam', () => {
    const onPersist = vi.fn();
    applyThemeWithWave('terra', { onPersist });
    expect(document.documentElement.dataset.theme).toBe('terra');
    expect(document.querySelector('.theme-wave-beam')).not.toBeNull();
    expect(onPersist).toHaveBeenCalledWith('terra');
  });

  it('with View Transitions, runs the callback and toggles the warp class', async () => {
    let finish!: () => void;
    const finished = new Promise<void>((r) => (finish = r));
    const startViewTransition = vi.fn((cb: () => void) => {
      cb();
      return { finished, ready: Promise.resolve(), updateCallbackDone: Promise.resolve(), skipTransition() {} };
    });
    // @ts-expect-error jsdom has no native impl
    document.startViewTransition = startViewTransition;

    const onPersist = vi.fn();
    applyThemeWithWave('nyx', { onPersist });

    expect(startViewTransition).toHaveBeenCalledOnce();
    expect(document.documentElement.dataset.theme).toBe('nyx');
    expect(document.documentElement.classList.contains('theme-warp')).toBe(true);
    expect(onPersist).toHaveBeenCalledWith('nyx');

    finish();
    await finished;
    await Promise.resolve();
    expect(document.documentElement.classList.contains('theme-warp')).toBe(false);
  });

  it("wave speed 'off' collapses into the instant no-beam path", () => {
    document.documentElement.dataset.waveSpeed = 'off';
    const onPersist = vi.fn();
    applyThemeWithWave('pyro', { onPersist });
    expect(document.documentElement.dataset.theme).toBe('pyro');
    expect(document.querySelector('.theme-wave-beam')).toBeNull();
    expect(onPersist).toHaveBeenCalledWith('pyro');
  });

  it("wave speed 'slow' still runs the beam (not the instant path)", () => {
    document.documentElement.dataset.waveSpeed = 'slow';
    const onPersist = vi.fn();
    applyThemeWithWave('terra', { onPersist });
    expect(document.documentElement.dataset.theme).toBe('terra');
    expect(document.querySelector('.theme-wave-beam')).not.toBeNull();
    expect(onPersist).toHaveBeenCalledWith('terra');
  });

  it('an unrecognised data-wave-speed value falls back to the default duration', () => {
    document.documentElement.dataset.waveSpeed = 'ludicrous';
    const onPersist = vi.fn();
    applyThemeWithWave('nyx', { onPersist });
    expect(document.documentElement.dataset.theme).toBe('nyx');
    expect(document.querySelector('.theme-wave-beam')).not.toBeNull();
    expect(onPersist).toHaveBeenCalledWith('nyx');
  });

  it('an explicit durationMs override wins even when data-wave-speed is off', () => {
    document.documentElement.dataset.waveSpeed = 'off';
    const onPersist = vi.fn();
    applyThemeWithWave('pyro', { onPersist, durationMs: 50 });
    expect(document.documentElement.dataset.theme).toBe('pyro');
    // durationMs=50 overrides the 'off' dataset value, so the beam path runs.
    expect(document.querySelector('.theme-wave-beam')).not.toBeNull();
    expect(onPersist).toHaveBeenCalledWith('pyro');
  });

  it('an explicit durationMs of 0 takes the instant no-beam path', () => {
    document.documentElement.dataset.waveSpeed = 'fast';
    const onPersist = vi.fn();
    applyThemeWithWave('terra', { onPersist, durationMs: 0 });
    expect(document.documentElement.dataset.theme).toBe('terra');
    expect(document.querySelector('.theme-wave-beam')).toBeNull();
    expect(onPersist).toHaveBeenCalledWith('terra');
  });
});
