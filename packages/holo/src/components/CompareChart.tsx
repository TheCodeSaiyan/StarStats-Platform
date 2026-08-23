'use client';

import React from 'react';

/**
 * Overlay comparison chart — two or more entities' stats drawn on one set of
 * axes so the difference is a shape, not a table you have to read twice.
 *
 * Two modes, because the answer depends on the question:
 *   `radar` — every stat at once, one polygon per entity. Good for "which
 *             of these is the all-rounder", useless for exact values.
 *   `bars`  — one grouped row per stat, aligned to a shared scale. Good for
 *             "how much faster", useless for overall shape.
 *
 * Each stat declares its own scale, because a comparison that normalises
 * speed against mass is meaningless. `invert` marks a stat where lower is
 * better, so the polygon still reads "bigger is stronger".
 */
const SERIES_COLOURS = ['var(--hot)', 'var(--fringe)', 'var(--warn)', 'var(--good)'];

export interface CompareStat {
  key: string;
  label: string;
  min: number;
  max: number;
  /** Lower is better — the polygon still reads "bigger is stronger". */
  invert?: boolean;
}

export interface CompareSeries {
  name: string;
  values: Record<string, number | null | undefined>;
}

export function CompareChart({
  mode = 'radar',
  stats = [],
  series = [],
  size = 300,
}: {
  mode?: 'radar' | 'bars';
  stats?: CompareStat[];
  series?: CompareSeries[];
  size?: number;
}) {
  if (!stats.length || !series.length) return null;
  const norm = (s: CompareStat, v: number | null | undefined) => {
    if (v == null) return 0;
    const t = (v - s.min) / (s.max - s.min || 1);
    const c = Math.max(0, Math.min(1, t));
    return s.invert ? 1 - c : c;
  };

  if (mode === 'bars') {
    return (
      <div className="hp-cmp-bars">
        {stats.map((s) => (
          <div className="row" key={s.key}>
            <span className="lab">{s.label}</span>
            <span className="track">
              {series.map((e, i) => (
                <span key={e.name} className="bar" style={{
                  width: (norm(s, e.values[s.key]) * 100).toFixed(1) + '%',
                  background: SERIES_COLOURS[i % SERIES_COLOURS.length],
                  opacity: 1 - i * 0.18,
                }} />
              ))}
            </span>
            <span className="vals">
              {series.map((e, i) => (
                <b key={e.name} style={{ color: SERIES_COLOURS[i % SERIES_COLOURS.length] }}>
                  {e.values[s.key] == null
                    ? '—'
                    : (e.values[s.key] as number).toLocaleString('en-GB')}
                </b>
              ))}
            </span>
          </div>
        ))}
      </div>
    );
  }

  const R = size / 2;
  const pad = 34;
  const rad = R - pad;
  const pt = (i: number, t: number): [number, number] => {
    const a = (i / stats.length) * Math.PI * 2 - Math.PI / 2;
    // Quantised to 3dp. `Math.cos`/`Math.sin` differ in the last ULP between
    // Node and the browser, and an SVG coordinate that differs by 1e-16 is a
    // hydration mismatch that regenerates the whole tree — the same trap the
    // Ring hit.
    const q = (n: number) => Math.round(n * 1000) / 1000;
    return [q(R + Math.cos(a) * rad * t), q(R + Math.sin(a) * rad * t)];
  };
  const poly = (e: CompareSeries) => stats.map((s, i) => pt(i, norm(s, e.values[s.key])).join(',')).join(' ');

  return (
    <div className="hp-cmp-radar">
      <svg viewBox={`0 0 ${size} ${size}`} role="img"
        aria-label={'Comparison of ' + series.map((s) => s.name).join(', ')}>
        {[0.25, 0.5, 0.75, 1].map((t) => (
          <polygon key={t} points={stats.map((s, i) => pt(i, t).join(',')).join(' ')}
            fill="none" stroke="rgba(var(--bR),var(--bG),var(--bB),.14)" strokeWidth="1" />
        ))}
        {stats.map((s, i) => {
          const [x, y] = pt(i, 1);
          return <line key={s.key} x1={R} y1={R} x2={x} y2={y} stroke="rgba(var(--bR),var(--bG),var(--bB),.12)" />;
        })}
        {series.map((e, i) => (
          <polygon key={e.name} points={poly(e)}
            fill={SERIES_COLOURS[i % SERIES_COLOURS.length]} fillOpacity={0.10}
            stroke={SERIES_COLOURS[i % SERIES_COLOURS.length]} strokeWidth="1.6" />
        ))}
        {stats.map((s, i) => {
          const [x, y] = pt(i, 1.16);
          return (
            <text key={s.key} x={x} y={y} fontSize="8.5" letterSpacing="1.4" textAnchor="middle"
              dominantBaseline="middle" fontFamily="var(--font-sans)"
              style={{ fill: 'color-mix(in oklab, var(--dim) 55%, var(--beam))' }}>
              {s.label.toUpperCase()}
            </text>
          );
        })}
      </svg>
    </div>
  );
}

/** Legend + the series picker that drives a `CompareChart`. */
export function CompareBar({
  options = [],
  selected = [],
  onToggle,
  max = 3,
  modes = ['radar', 'bars'],
  mode,
  onMode,
}: {
  options?: string[];
  selected?: string[];
  onToggle?: (o: string) => void;
  max?: number;
  modes?: ('radar' | 'bars')[];
  mode?: 'radar' | 'bars';
  onMode?: (m: 'radar' | 'bars') => void;
}) {
  return (
    <div className="hp-cmp-bar">
      <span className="lab">Compare</span>
      <div className="picks">
        {options.map((o) => {
          const i = selected.indexOf(o);
          const on = i > -1;
          return (
            <button key={o} type="button" aria-pressed={on} disabled={!on && selected.length >= max}
              onClick={() => onToggle && onToggle(o)}
              style={on ? { color: SERIES_COLOURS[i % SERIES_COLOURS.length], borderColor: SERIES_COLOURS[i % SERIES_COLOURS.length] } : undefined}>
              {on ? <i style={{ background: SERIES_COLOURS[i % SERIES_COLOURS.length] }} /> : null}
              {o}
            </button>
          );
        })}
      </div>
      <span className="sp" />
      <div className="modes">
        {modes.map((m) => (
          <button key={m} type="button" aria-pressed={m === mode} onClick={() => onMode && onMode(m)}>{m}</button>
        ))}
      </div>
    </div>
  );
}
