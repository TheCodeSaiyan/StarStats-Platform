import React from 'react';

/**
 * Empty state: the beam flatlines. Never a blank plane.
 *
 * Replaces the flat product's `NoSignal`. The waveform is one of the few real
 * inline SVG glyphs in the system (1.4 stroke, round caps) — there is no icon
 * font in this brand and no emoji anywhere.
 *
 * The reason phrasing matters: error copy stays literal even though the chrome
 * is in-universe. "The game didn't write this telemetry" tells a reader
 * something they can act on; "no signal" alone does not.
 */
export type FlatlineReason = 'no-data' | 'no-telemetry' | 'no-signal';

const HINT: Record<FlatlineReason, string> = {
  'no-data': 'No activity recorded in this window.',
  'no-telemetry':
    'The game didn’t write this telemetry for the selected session.',
  'no-signal': 'No emitter is streaming to this projection.',
};

export interface FlatlineProps {
  title?: React.ReactNode;
  reason?: FlatlineReason;
  /** Overrides the reason-derived hint. */
  hint?: React.ReactNode;
  action?: React.ReactNode;
  /**
   * Compact form for a per-element empty state inside a Plane, rather than the
   * full-surface state. (Addition — see gap A3: the flat system had a per-tile
   * `<NoSignal compact />` and the projection had only a whole-screen form.)
   */
  compact?: boolean;
}

export function Flatline({
  title = 'No signal in this window',
  reason = 'no-data',
  hint,
  action,
  compact = false,
}: FlatlineProps) {
  const resolved = hint ?? HINT[reason];
  if (compact) {
    return (
      <div className="hp-empty" role="status" aria-live="polite">
        {title}
      </div>
    );
  }
  return (
    <div className="hp-nosig" role="status" aria-live="polite">
      <svg viewBox="0 0 108 34" aria-hidden="true">
        <path
          d="M0 17 L18 17 L23 6 L28 28 L33 12 L38 21 L43 17 L58 17"
          fill="none"
          style={{ stroke: 'var(--beam)' }}
          strokeWidth="1.4"
          opacity="0.55"
          strokeLinecap="round"
        />
        <path
          d="M62 17 L76 17 M84 17 L108 17"
          fill="none"
          style={{ stroke: 'var(--hot)' }}
          strokeWidth="1.4"
          strokeDasharray="3 4"
          strokeLinecap="round"
        />
      </svg>
      <span className="t">{title}</span>
      {resolved ? <span className="h">{resolved}</span> : null}
      {action}
    </div>
  );
}
