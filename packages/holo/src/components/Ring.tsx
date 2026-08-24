'use client';

import React from 'react';

const R = 280;

/**
 * Round a coordinate before it becomes part of an attribute string.
 *
 * SSR HYDRATION DEPENDS ON THIS. `Math.cos`/`Math.sin` are not required to be
 * correctly rounded, and Node and the browser can differ in the last unit in the
 * last place. Serialised at full precision that surfaces as a `d` attribute
 * whose 15th significant figure disagrees between the server HTML and the
 * client render, and React reports a hydration mismatch on every paint of the
 * ring. Three decimals on a 560-unit viewBox is far below a device pixel, and
 * it makes the markup materially smaller as a side effect.
 */
const q = (n: number): number => Math.round(n * 1000) / 1000;

const pt = (r: number, a: number): [number, number] => [
  q(R + Math.cos(a) * r),
  q(R + Math.sin(a) * r),
];

/**
 * The ring: the projection's navigation and its chart at once.
 *
 * `segments` — lens shares around the circumference (overview).
 * `bars`     — a 12-step radial histogram (non-spatial lens detail).
 * `map`      — the same ring becomes a transit graph of places (spatial lens).
 *
 * Spatial lenses (travel) use `map`; everything else uses `bars` — that switch
 * is the point, not a preference. Keep `segments` supplied even in bars/map
 * mode so the reticle can still point at the active lens.
 *
 * CAPACITY: reads to about 24 bars / 12 segments / 8 map nodes. Past that,
 * aggregate — the ring is a shape, not a table.
 */
export interface RingSegment {
  name: string;
  /** Fraction of the circumference, 0–1. Shares across the set should sum to 1. */
  share: number;
}

export interface RingNode {
  /** Node name; also the value handed to `onSelectNode`. */
  n: string;
  /** Angle in DEGREES. Use `layoutMapNodes()` rather than hand-placing. */
  a: number;
  /** Dot radius, scaled from visit weight. */
  r: number;
  ctx?: string;
}

/**
 * A place the reader has never been (gap B3): drawn as a dim labelled tick at
 * the rim instead of an empty arc, so dead space carries information rather
 * than reading as a rendering fault.
 */
export interface RingTick {
  label: string;
  /** Angle in DEGREES. */
  a: number;
}

export interface RingProps {
  mode?: 'segments' | 'bars' | 'map';
  segments?: RingSegment[];
  activeIndex?: number;
  bars?: number[];
  nodes?: RingNode[];
  /** `[fromIndex, toIndex, strokeWidth]` into `nodes`. */
  links?: [number, number, number][];
  ticks?: RingTick[];
  onSelectSegment?: (index: number) => void;
  onSelectNode?: (name: string) => void;
  /** Compact px box for the tray — draws its own centre figure. */
  size?: number;
  label?: string;
  value?: React.ReactNode;
}

export function Ring({
  mode = 'segments',
  segments = [],
  activeIndex = -1,
  bars = [],
  nodes = [],
  links = [],
  ticks = [],
  onSelectSegment,
  onSelectNode,
  size,
  label,
  value,
}: RingProps) {
  const segPaths: {
    i: number;
    d: string;
    lx: number;
    ly: number;
    name: string;
  }[] = [];
  if (mode === 'segments') {
    let a = -Math.PI / 2;
    const gap = 0.055;
    segments.forEach((s, i) => {
      const sw = s.share * Math.PI * 2 - gap;
      const [x1, y1] = pt(222, a);
      const [x2, y2] = pt(222, a + sw);
      const [lx, ly] = pt(198, a + sw / 2);
      segPaths.push({
        i,
        d: `M${x1} ${y1} A222 222 0 ${sw > Math.PI ? 1 : 0} 1 ${x2} ${y2}`,
        lx,
        ly,
        name: s.name,
      });
      a += sw + gap;
    });
  }

  // Reticle angle for the active lens.
  let retAngle: number | null = null;
  if (activeIndex > -1 && segments.length) {
    let a = -Math.PI / 2;
    for (let j = 0; j < activeIndex; j++) a += segments[j].share * Math.PI * 2;
    a += (segments[activeIndex].share * Math.PI * 2) / 2;
    retAngle = q(((a + Math.PI / 2) * 180) / Math.PI);
  }

  const maxBar = bars.length ? Math.max(...bars) : 0;
  const pos = nodes.map((n) => {
    const a = (n.a * Math.PI) / 180;
    return { ...n, x: q(R + Math.cos(a) * 196), y: q(R + Math.sin(a) * 196) };
  });

  const compact = size != null;
  return (
    <div
      className={compact ? 'hp-ringwrap hp-ringwrap--compact' : 'hp-ringwrap'}
      style={compact ? { width: size, height: size } : undefined}
    >
      <svg viewBox="0 0 560 560">
        <g className="hp-tickring">
          <circle
            cx={R}
            cy={R}
            r="256"
            fill="none"
            stroke="rgba(var(--bR),var(--bG),var(--bB),.1)"
            strokeWidth="14"
            strokeDasharray="1 8"
          />
        </g>
        <g className="hp-tickring rev">
          <circle
            cx={R}
            cy={R}
            r="236"
            fill="none"
            stroke="rgba(var(--bR),var(--bG),var(--bB),.14)"
            strokeWidth="6"
            strokeDasharray="2 22"
          />
        </g>
        <circle
          cx={R}
          cy={R}
          r="246"
          fill="none"
          stroke="rgba(var(--bR),var(--bG),var(--bB),.12)"
        />
        <circle
          cx={R}
          cy={R}
          r="196"
          fill="none"
          stroke="rgba(var(--fR),var(--fG),var(--fB),.18)"
          strokeDasharray="3 10"
        />

        {mode === 'segments' &&
          segPaths.map((s) => (
            <g key={s.i}>
              {/* A 9px stroke is a ~9px target. The hit path is invisible, 34px
                  wide, and carries the interaction so the visual can stay thin. */}
              <path
                className="hp-seg"
                d={s.d}
                fill="none"
                style={{ stroke: 'var(--beam)' }}
                strokeWidth="9"
                opacity="0.55"
                pointerEvents="none"
              />
              <path
                className="hp-seghit"
                d={s.d}
                fill="none"
                stroke="transparent"
                strokeWidth="34"
                role="button"
                tabIndex={0}
                aria-label={`Open ${s.name}`}
                aria-current={s.i === activeIndex ? 'true' : undefined}
                onClick={() => onSelectSegment && onSelectSegment(s.i)}
                onKeyDown={(e) => {
                  if (e.key === 'Enter' || e.key === ' ') {
                    e.preventDefault();
                    onSelectSegment && onSelectSegment(s.i);
                  }
                }}
              />
              <text
                x={s.lx}
                y={s.ly}
                style={{
                  fill: 'color-mix(in oklab, var(--dim) 55%, var(--beam))',
                }}
                // The label is a SIBLING of the hit band, not inside it, so a
                // tap that lands on the word does nothing at all — and the word
                // is the obvious thing to aim at. Passing it through puts those
                // taps on the band underneath.
                pointerEvents="none"
                fontSize="9"
                letterSpacing="2.4"
                textAnchor="middle"
                dominantBaseline="middle"
                fontFamily="var(--font-sans)"
              >
                {s.name.toUpperCase()}
              </text>
            </g>
          ))}

        {mode === 'bars' &&
          bars.map((v, i) => {
            const a = (i / bars.length) * Math.PI * 2 - Math.PI / 2;
            const [x1, y1] = pt(186, a);
            const [x2, y2] = pt(186 + (v / 100) * 72, a);
            return (
              <line
                key={i}
                x1={x1}
                y1={y1}
                x2={x2}
                y2={y2}
                strokeWidth="7"
                strokeLinecap="round"
                style={{ stroke: v === maxBar ? 'var(--hot)' : 'var(--beam)' }}
                opacity={(0.28 + v / 160).toFixed(2)}
              />
            );
          })}

        {mode === 'map' &&
          links.map(([i, j, w], k) =>
            pos[i] && pos[j] ? (
              <path
                key={k}
                d={`M${pos[i].x} ${pos[i].y} Q ${R} ${R} ${pos[j].x} ${pos[j].y}`}
                fill="none"
                style={{ stroke: 'var(--beam)' }}
                strokeWidth={w}
                opacity="0.4"
              />
            ) : null,
          )}
        {mode === 'map' &&
          pos.map((p) => (
            <g
              key={p.n}
              className="hp-mapnode"
              role="button"
              tabIndex={0}
              aria-label={`Open ${p.n}`}
              onClick={() => onSelectNode && onSelectNode(p.n)}
              onKeyDown={(e) => {
                if (e.key === 'Enter' || e.key === ' ') {
                  e.preventDefault();
                  onSelectNode && onSelectNode(p.n);
                }
              }}
            >
              {/* Invisible 44px-equivalent target behind the visible dot. */}
              <circle
                cx={p.x}
                cy={p.y}
                r={Math.max(p.r + 14, 24)}
                fill="transparent"
              />
              <circle
                cx={p.x}
                cy={p.y}
                r={p.r}
                strokeWidth="1"
                style={{
                  fill: 'rgba(var(--bR),var(--bG),var(--bB),.28)',
                  stroke: 'var(--beam)',
                }}
              />
              <text
                x={p.x}
                y={p.y - p.r - 9}
                style={{ fill: 'var(--hot)' }}
                fontSize="9"
                letterSpacing="1.8"
                textAnchor="middle"
                fontFamily="var(--font-sans)"
              >
                {p.n.toUpperCase()}
              </text>
            </g>
          ))}

        {/* Unvisited systems (gap B3). Not interactive — there is nothing to
            open — so they are marks, not targets: a dim tick plus a name. */}
        {mode === 'map' &&
          ticks.map((t) => {
            const a = (t.a * Math.PI) / 180;
            const [x1, y1] = pt(232, a);
            const [x2, y2] = pt(244, a);
            const [tx, ty] = pt(258, a);
            return (
              <g key={t.label} className="hp-maptick" aria-hidden="true">
                <line
                  x1={x1}
                  y1={y1}
                  x2={x2}
                  y2={y2}
                  strokeWidth="1"
                  style={{ stroke: 'var(--dim)' }}
                  opacity="0.7"
                />
                <text
                  x={tx}
                  y={ty}
                  style={{ fill: 'var(--dim)' }}
                  fontSize="8"
                  letterSpacing="1.6"
                  textAnchor="middle"
                  dominantBaseline="middle"
                  fontFamily="var(--font-sans)"
                >
                  {t.label.toUpperCase()}
                </text>
              </g>
            );
          })}

        {compact && value != null ? (
          <g>
            <text
              x={R}
              y={R - 6}
              style={{ fill: 'var(--hot)' }}
              fontSize="76"
              fontWeight="600"
              letterSpacing="-2"
              textAnchor="middle"
              dominantBaseline="middle"
              fontFamily="var(--font-sans)"
            >
              {value}
            </text>
            {label ? (
              <text
                x={R}
                y={R + 54}
                style={{
                  fill: 'color-mix(in oklab, var(--dim) 55%, var(--beam))',
                }}
                fontSize="20"
                letterSpacing="9"
                textAnchor="middle"
                fontFamily="var(--font-sans)"
              >
                {String(label).toUpperCase()}
              </text>
            ) : null}
          </g>
        ) : null}

        <g
          className="hp-reticle"
          opacity={retAngle == null ? 0 : 1}
          style={
            retAngle == null
              ? undefined
              : { transform: `rotate(${retAngle}deg)` }
          }
        >
          <path d="M280 12 l9 15 h-18 z" style={{ fill: 'var(--hot)' }} />
          <line x1="280" y1="30" x2="280" y2="46" style={{ stroke: 'var(--hot)' }} />
        </g>
      </svg>
    </div>
  );
}
