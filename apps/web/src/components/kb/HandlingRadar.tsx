import React from 'react';
import type { Quantiles } from '@/lib/kb-stats';

export interface RadarAxis {
  label: string;
  value: number;
  q: Quantiles;
}

/** Normalised position 0..1 of `x` within [min,max]. */
function frac(x: number, q: Quantiles): number {
  if (q.max === q.min) return 0.5;
  return Math.max(0.04, Math.min(1, (x - q.min) / (q.max - q.min)));
}

/**
 * SVG radar of the entity's profile vs the class median. Each axis is
 * scaled to that metric's class min–max so the polygon shape reads as
 * "where this entity sits in its class". Renders nothing for <3 axes.
 */
export function HandlingRadar({ axes, size = 260 }: { axes: RadarAxis[]; size?: number }) {
  if (axes.length < 3) return null;
  const c = size / 2;
  const R = size / 2 - 34;
  const pt = (i: number, f: number): string => {
    const ang = -Math.PI / 2 + (i * 2 * Math.PI) / axes.length;
    return `${(c + R * f * Math.cos(ang)).toFixed(1)},${(c + R * f * Math.sin(ang)).toFixed(1)}`;
  };
  const shipPoly = axes.map((a, i) => pt(i, frac(a.value, a.q))).join(' ');
  const medPoly = axes.map((a, i) => pt(i, frac(a.q.p50, a.q))).join(' ');
  const rings = [0.25, 0.5, 0.75, 1].map((g) => (
    <polygon key={g} points={axes.map((_, i) => pt(i, g)).join(' ')} fill="none" stroke="rgba(255,255,255,0.07)" />
  ));
  return (
    <svg viewBox={`0 0 ${size} ${size}`} width={size} height={size} role="img" aria-label="Class profile radar">
      {rings}
      {axes.map((a, i) => {
        const ang = -Math.PI / 2 + (i * 2 * Math.PI) / axes.length;
        const lx = c + (R + 16) * Math.cos(ang);
        const ly = c + (R + 16) * Math.sin(ang);
        const anchor = Math.abs(Math.cos(ang)) < 0.3 ? 'middle' : Math.cos(ang) > 0 ? 'start' : 'end';
        return (
          <text key={a.label} x={lx.toFixed(1)} y={(ly + 3).toFixed(1)} fontSize={10} fill="var(--fg-muted)" textAnchor={anchor}>
            {a.label}
          </text>
        );
      })}
      <polygon points={medPoly} fill="none" stroke="rgba(255,255,255,0.28)" strokeWidth={1.2} strokeDasharray="3 3" />
      {/* The beam, not a literal amber. This polygon IS the entity you are
          reading, so it takes `--hot` and glows; hardcoding the flat accent
          meant the one shape on the page representing "you are here" ignored
          the calibration. */}
      <polygon
        points={shipPoly}
        fill="rgba(var(--bR), var(--bG), var(--bB), 0.14)"
        stroke="var(--hot)"
        strokeWidth={1.6}
      />
    </svg>
  );
}
