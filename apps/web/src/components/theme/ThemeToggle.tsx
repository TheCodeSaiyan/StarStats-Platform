'use client';

import React, { useEffect, useRef, useState } from 'react';
import { THEME_ACCENT } from '@/lib/theme-accents';
import { applyThemeWithWave } from '@/lib/theme-transition';
import { persistThemeAction } from '@/app/_actions/theme';
import { type Theme } from '@/lib/theme';

const THEMES_UI: ReadonlyArray<{ id: Theme; name: string }> = [
  { id: 'stanton', name: 'Stanton' },
  { id: 'pyro', name: 'Pyro' },
  { id: 'terra', name: 'Terra' },
  { id: 'nyx', name: 'Nyx' },
];

/**
 * Compact TopBar theme control. The accent dot inherits `var(--accent)` so
 * it is correct in any theme with no client knowledge. Clicking opens a
 * popover of the four themes; each fires the in-place wave + persists. With
 * JS off, the trigger is a plain link to the Settings theme card.
 */
export function ThemeToggle() {
  const [open, setOpen] = useState(false);
  const [current, setCurrent] = useState<Theme | null>(null);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    setCurrent((document.documentElement.dataset.theme as Theme) ?? null);
  }, []);

  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false);
    };
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setOpen(false);
    };
    document.addEventListener('mousedown', onDown);
    document.addEventListener('keydown', onKeyDown);
    return () => {
      document.removeEventListener('mousedown', onDown);
      document.removeEventListener('keydown', onKeyDown);
    };
  }, [open]);

  function pick(theme: Theme) {
    setCurrent(theme);
    setOpen(false);
    applyThemeWithWave(theme, { onPersist: persistThemeAction });
  }

  return (
    <div className="theme-toggle" ref={ref}>
      <a
        href="/settings#theme"
        className="theme-toggle__btn"
        role="button"
        aria-haspopup="menu"
        aria-expanded={open}
        aria-label="Change theme"
        onClick={(e) => {
          e.preventDefault();
          setOpen((o) => !o);
        }}
        onKeyDown={(e) => {
          if (e.repeat) return;
          if (e.key === ' ' || e.key === 'Spacebar') {
            e.preventDefault();
            setOpen((o) => !o);
          }
        }}
      >
        <span className="theme-toggle__dot" aria-hidden="true" />
      </a>
      {open && (
        <div className="theme-toggle__menu" role="menu" aria-label="Theme">
          {THEMES_UI.map((t) => (
            <button
              key={t.id}
              type="button"
              role="menuitemradio"
              aria-checked={t.id === current}
              className="theme-toggle__item"
              onClick={() => pick(t.id)}
            >
              <span
                className="theme-toggle__swatch"
                style={{ background: THEME_ACCENT[t.id] }}
                aria-hidden="true"
              />
              {t.name}
              {t.id === current && (
                <span className="theme-toggle__check" aria-hidden="true">
                  ✓
                </span>
              )}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
