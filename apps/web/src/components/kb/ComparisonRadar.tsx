import React from 'react';

export interface RadarSeriesView {
  slug: string;
  name: string;
  color: string;
  values: number[]; // 0..1 per axis, aligned to axisLabels
}

/**
 * Multi-series radar: one polygon per ship over shared axes, with a
 * legend. Each `values[i]` is a 0..1 fraction already scaled to the
 * compared set (see `buildComparisonRadar`). Renders nothing for <3 axes
 * (a radar needs a polygon).
 */
export function ComparisonRadar({
  axisLabels,
  series,
  size = 280,
}: {
  axisLabels: string[];
  series: RadarSeriesView[];
  size?: number;
}) {
  if (axisLabels.length < 3) return null;
  const c = size / 2;
  const R = size / 2 - 38;
  const n = axisLabels.length;
  const pt = (i: number, f: number) => {
    const ang = -Math.PI / 2 + (i * 2 * Math.PI) / n;
    return `${(c + R * f * Math.cos(ang)).toFixed(1)},${(c + R * f * Math.sin(ang)).toFixed(1)}`;
  };
  const rings = [0.25, 0.5, 0.75, 1].map((g) => (
    <polygon key={g} points={axisLabels.map((_, i) => pt(i, g)).join(' ')} fill="none" stroke="rgba(255,255,255,0.06)" />
  ));
  return (
    <div style={{ display: 'flex', gap: 20, alignItems: 'center', flexWrap: 'wrap', justifyContent: 'center' }}>
      <svg viewBox={`0 0 ${size} ${size}`} width={size} height={size} role="img" aria-label="Multi-ship comparison radar">
        {rings}
        {axisLabels.map((lab, i) => {
          const ang = -Math.PI / 2 + (i * 2 * Math.PI) / n;
          const lx = c + (R + 16) * Math.cos(ang);
          const ly = c + (R + 16) * Math.sin(ang);
          const anchor = Math.abs(Math.cos(ang)) < 0.3 ? 'middle' : Math.cos(ang) > 0 ? 'start' : 'end';
          return (
            <text key={lab} x={lx.toFixed(1)} y={(ly + 3).toFixed(1)} fontSize={10} fill="var(--fg-muted)" textAnchor={anchor}>
              {lab}
            </text>
          );
        })}
        {series.map((s) => (
          <polygon
            key={s.slug}
            data-series={s.slug}
            points={s.values.map((f, i) => pt(i, Math.max(0.04, f))).join(' ')}
            fill={`${s.color}22`}
            stroke={s.color}
            strokeWidth={2}
          />
        ))}
      </svg>
      <div style={{ fontSize: 12, color: 'var(--fg)', lineHeight: 1.9 }}>
        {series.map((s) => (
          <div key={s.slug} style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
            {/* A hairline rule in the series colour, not a filled rounded
                swatch — the shape rules hold inside the series exception. */}
            <span
              style={{
                width: 12,
                height: 0,
                borderTop: `2px solid ${s.color}`,
                display: 'inline-block',
              }}
            />
            {s.name}
          </div>
        ))}
      </div>
    </div>
  );
}
