import React from 'react';

/**
 * The two chart shapes the projection uses. `wave` is a continuous line over a
 * soft ghost of itself; `ribbon` is a bar field with the recent buckets lit.
 *
 * `values` IS REQUIRED HERE, and that is a deliberate departure from the kit.
 *
 * The kit's version generates a deterministic pseudo-random series when
 * `values` is absent, because it is a mock with no data behind it. Shipping
 * that in the product would draw a chart of nothing and present it as the
 * reader's own history — a fabricated figure on the most personal screen in the
 * app, and the system's own rule is that missing is `—`, never invented. So the
 * seeded fallback is gone: a caller with no series renders no trace.
 */
export interface TraceProps {
  cap?: React.ReactNode;
  mode?: 'wave' | 'ribbon';
  /** The real series. No values, no chart. */
  values: number[];
  /** Index past which bars read as "recent" and take the hot tone. */
  lit?: number;
}

export function Trace({ cap, mode = 'wave', values, lit }: TraceProps) {
  if (!values || values.length === 0) return null;

  // Normalise to the viewBox's 0–58 band so a caller can pass raw counts.
  const peak = Math.max(...values, 1);
  const scaled = values.map((v) => (v / peak) * 58);

  let body: React.ReactNode;
  if (mode === 'ribbon') {
    const litFrom = lit ?? Math.floor(scaled.length * 0.78);
    const w = 560 / scaled.length;
    body = scaled.map((h, i) => (
      <rect
        key={i}
        x={i * w}
        y={70 - h}
        width={Math.max(1, w - 3)}
        height={Math.max(1, h)}
        style={{ fill: i > litFrom ? 'var(--hot)' : 'var(--beam)' }}
        opacity={(0.3 + h / 90).toFixed(2)}
      />
    ));
  } else {
    const step = scaled.length > 1 ? 560 / (scaled.length - 1) : 560;
    // Quantised: Node and the browser can disagree in the last ULP, and a
    // mismatched `d` attribute tears the tree down on hydration.
    let d = `M0 ${(68 - scaled[0]).toFixed(1)}`;
    scaled.forEach((v, i) => {
      d += ` L${(i * step).toFixed(1)} ${(68 - v).toFixed(1)}`;
    });
    body = (
      <>
        <path
          d={d}
          fill="none"
          style={{ stroke: 'var(--beam)' }}
          strokeWidth="4"
          opacity="0.14"
        />
        <path
          d={d}
          fill="none"
          style={{ stroke: 'var(--beam)' }}
          strokeWidth="1.4"
        />
      </>
    );
  }

  return (
    <div className="hp-graf">
      {cap ? <div className="hp-grafcap">{cap}</div> : null}
      <svg viewBox="0 0 560 76" preserveAspectRatio="none" role="img" aria-label={typeof cap === 'string' ? cap : 'Trace'}>
        {body}
      </svg>
    </div>
  );
}
