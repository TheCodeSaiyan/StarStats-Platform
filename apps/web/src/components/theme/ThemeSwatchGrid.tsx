'use client';

// Classic-JSX-runtime vitest needs the explicit React import.
import React, { useState } from 'react';
import { THEME_ACCENT } from '@/lib/theme-accents';
import { applyThemeWithWave } from '@/lib/theme-transition';
import { persistThemeAction } from '@/app/_actions/theme';
import { type Theme } from '@/lib/theme';

interface ThemeMeta {
  id: Theme;
  name: string;
  subtitle: string;
}

const THEME_META: readonly ThemeMeta[] = [
  { id: 'stanton', name: 'Stanton', subtitle: 'Default · warm amber' },
  { id: 'pyro', name: 'Pyro', subtitle: 'Molten coral · aggressive' },
  { id: 'terra', name: 'Terra', subtitle: 'Cool teal · clinical' },
  { id: 'nyx', name: 'Nyx', subtitle: 'Light · deep violet' },
];

/**
 * Theme swatch grid. Progressive enhancement: the buttons live inside the
 * server-action `<form>` so with JS OFF a click submits and reloads (the
 * legacy path). With JS ON we intercept, run the in-place wave, and persist
 * without navigating.
 */
export function ThemeSwatchGrid({
  activeTheme,
  themeAction,
}: {
  activeTheme: Theme;
  themeAction: (formData: FormData) => void | Promise<void>;
}) {
  const [active, setActive] = useState<Theme>(activeTheme);

  return (
    <form action={themeAction} style={{ margin: 0 }}>
      <div
        data-rspgrid="4"
        style={{
          display: 'grid',
          gridTemplateColumns: 'repeat(4, minmax(0, 1fr))',
          gap: 12,
        }}
      >
        {THEME_META.map((t) => {
          const isActive = t.id === active;
          return (
            <button
              key={t.id}
              type="submit"
              name="theme"
              value={t.id}
              className="ss-theme-swatch"
              data-active={isActive ? 'true' : undefined}
              aria-pressed={isActive}
              aria-label={`Switch to ${t.name} theme`}
              onClick={(e) => {
                // JS present → animate in place instead of submitting.
                e.preventDefault();
                setActive(t.id);
                applyThemeWithWave(t.id, { onPersist: persistThemeAction });
              }}
              style={{ cursor: 'pointer', font: 'inherit', textAlign: 'left' }}
            >
              <span
                style={{ fontWeight: 600, fontSize: 14, letterSpacing: '-0.01em' }}
              >
                {t.name}
              </span>
              <span style={{ display: 'block', fontSize: 11, opacity: 0.7 }}>
                {t.subtitle}
              </span>
              <span
                aria-hidden="true"
                style={{
                  display: 'block',
                  marginTop: 10,
                  height: 8,
                  borderRadius: 4,
                  background: THEME_ACCENT[t.id],
                }}
              />
            </button>
          );
        })}
      </div>
    </form>
  );
}
