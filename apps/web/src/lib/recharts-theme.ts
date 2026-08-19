/**
 * Bridge between the `ss-*` CSS-variable token system and recharts'
 * prop-driven colour API.
 *
 * Recharts components take literal hex (or rgba) strings as fills /
 * strokes — they don't read CSS vars at paint time. We resolve the
 * computed style of `:root` here so swapping themes
 * (`[data-theme="pyro"]` etc.) re-renders charts with the new accent.
 */

'use client';

import { useEffect, useState } from 'react';

export interface ChartTheme {
  accent: string;
  accentSoft: string;
  accentGlow: string;
  fg: string;
  fgMuted: string;
  fgDim: string;
  ok: string;
  warn: string;
  danger: string;
  info: string;
  border: string;
  surface: string;
  // Discrete heatmap buckets — surfaced separately because the
  // tokens.css `--grid-1..4` ladder is non-linear (eye-tuned).
  grid: [string, string, string, string, string];
}

const FALLBACK: ChartTheme = {
  accent: '#E8A23C',
  accentSoft: 'rgba(232, 162, 60, 0.14)',
  accentGlow: 'rgba(232, 162, 60, 0.28)',
  fg: '#ECE7DD',
  fgMuted: '#A09A8E',
  fgDim: '#6B6760',
  ok: '#74C68A',
  warn: '#E8C53C',
  danger: '#E8674C',
  info: '#6FA8E8',
  border: 'rgba(255, 230, 200, 0.07)',
  surface: '#1A1820',
  grid: [
    'rgba(255, 230, 200, 0.04)',
    'rgba(232, 162, 60, 0.18)',
    'rgba(232, 162, 60, 0.38)',
    'rgba(232, 162, 60, 0.62)',
    'rgba(232, 162, 60, 0.92)',
  ],
};

function readVar(root: HTMLElement, name: string, fallback: string): string {
  const v = getComputedStyle(root).getPropertyValue(name).trim();
  return v.length > 0 ? v : fallback;
}

function readTheme(): ChartTheme {
  if (typeof document === 'undefined') return FALLBACK;
  const root = document.documentElement;
  return {
    accent: readVar(root, '--accent', FALLBACK.accent),
    accentSoft: readVar(root, '--accent-soft', FALLBACK.accentSoft),
    accentGlow: readVar(root, '--accent-glow', FALLBACK.accentGlow),
    fg: readVar(root, '--fg', FALLBACK.fg),
    fgMuted: readVar(root, '--fg-muted', FALLBACK.fgMuted),
    fgDim: readVar(root, '--fg-dim', FALLBACK.fgDim),
    ok: readVar(root, '--ok', FALLBACK.ok),
    warn: readVar(root, '--warn', FALLBACK.warn),
    danger: readVar(root, '--danger', FALLBACK.danger),
    info: readVar(root, '--info', FALLBACK.info),
    border: readVar(root, '--border', FALLBACK.border),
    surface: readVar(root, '--surface', FALLBACK.surface),
    grid: [
      readVar(root, '--grid-empty', FALLBACK.grid[0]),
      readVar(root, '--grid-1', FALLBACK.grid[1]),
      readVar(root, '--grid-2', FALLBACK.grid[2]),
      readVar(root, '--grid-3', FALLBACK.grid[3]),
      readVar(root, '--grid-4', FALLBACK.grid[4]),
    ],
  };
}

export function useChartTheme(): ChartTheme {
  const [theme, setTheme] = useState<ChartTheme>(FALLBACK);
  useEffect(() => {
    setTheme(readTheme());
    const root = document.documentElement;
    const observer = new MutationObserver(() => setTheme(readTheme()));
    observer.observe(root, { attributes: true, attributeFilter: ['data-theme'] });
    return () => observer.disconnect();
  }, []);
  return theme;
}
