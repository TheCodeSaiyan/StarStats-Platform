'use client';

// Classic-JSX-runtime vitest needs the explicit React import.
import React, { useState } from 'react';
import { persistWaveSpeedAction } from '@/app/_actions/wave-speed';
import { WAVE_SPEEDS, type WaveSpeed } from '@/lib/wave-speed';

const LABELS: Readonly<Record<WaveSpeed, string>> = {
  off: 'Off',
  slow: 'Slow',
  normal: 'Normal',
  fast: 'Fast',
};

/**
 * Theme-switch wave animation speed control. Progressive enhancement,
 * mirroring `ThemeSwatchGrid`: the buttons live inside the
 * server-action `<form>` so with JS OFF a click submits and reloads
 * (the legacy path, handled by the page's `waveSpeedAction`). With JS
 * ON we intercept, stamp the new speed on `<html data-wave-speed>`
 * immediately (so the next theme-switch wave in this tab picks it up
 * without a reload), and persist without navigating.
 *
 * Unlike the theme swatches, picking a speed doesn't itself replay a
 * wave — there's nothing to preview until the next theme switch — so
 * this has no `applyThemeWithWave` call.
 */
export function WaveSpeedControl({
  initialSpeed,
  waveSpeedAction,
}: {
  initialSpeed: WaveSpeed;
  waveSpeedAction: (formData: FormData) => void | Promise<void>;
}) {
  const [active, setActive] = useState<WaveSpeed>(initialSpeed);

  return (
    <form action={waveSpeedAction} style={{ margin: 0 }}>
      <div
        role="group"
        aria-label="Theme-switch wave speed"
        style={{ display: 'flex', gap: 8, flexWrap: 'wrap' }}
      >
        {WAVE_SPEEDS.map((speed) => {
          const isActive = speed === active;
          return (
            <button
              key={speed}
              type="submit"
              name="wave_speed"
              value={speed}
              className="ss-btn"
              aria-pressed={isActive}
              data-active={isActive ? 'true' : undefined}
              onClick={(e) => {
                // JS present → apply + persist without a page reload.
                e.preventDefault();
                setActive(speed);
                if (typeof document !== 'undefined') {
                  document.documentElement.dataset.waveSpeed = speed;
                }
                void persistWaveSpeedAction(speed);
              }}
              style={{
                fontWeight: isActive ? 600 : 400,
                borderColor: isActive ? 'var(--border-strong)' : undefined,
              }}
            >
              {LABELS[speed]}
            </button>
          );
        })}
      </div>
    </form>
  );
}
