import React from 'react';

/**
 * Shared "No Telemetry Signal Found" empty state — a HUD-styled scope
 * graphic instead of a blank box. Server- and client-safe (pure
 * presentational, no hooks), so it drops into server-rendered widget
 * bodies and client widgets alike.
 *
 * Two reasons, because "we have no data for this window" and "the game
 * never wrote this telemetry" are different truths and the hint should
 * say which:
 *   - `no-data`      → the range is simply empty (fly more).
 *   - `no-telemetry` → the log didn't carry this signal that session.
 */
export type NoSignalReason = 'no-data' | 'no-telemetry';

const REASON_HINT: Record<NoSignalReason, string> = {
  'no-data': 'No activity recorded in this window.',
  'no-telemetry': 'The game didn’t write this telemetry for the selected session.',
};

export interface NoSignalProps {
  title?: string;
  reason?: NoSignalReason;
  /** Overrides the reason-derived hint. Pass '' to suppress the hint. */
  hint?: string;
  /** Tile-sized variant (smaller scope + type). */
  compact?: boolean;
  className?: string;
}

export function NoSignal({
  title = 'No Telemetry Signal Found',
  reason = 'no-data',
  hint,
  compact = false,
  className,
}: NoSignalProps) {
  const resolvedHint = hint ?? REASON_HINT[reason];
  const classes = ['hud-nosignal', compact ? 'hud-nosignal--compact' : '', className]
    .filter(Boolean)
    .join(' ');

  return (
    <div className={classes} role="status" aria-live="polite">
      <svg
        className="hud-nosignal__scope"
        viewBox="0 0 64 40"
        role="img"
        aria-label="Signal lost"
        focusable="false"
      >
        <rect x="1" y="1" width="62" height="38" rx="4" className="hud-nosignal__frame" />
        {/* a waveform that decays into a dashed flatline with a gap */}
        <path
          className="hud-nosignal__wave"
          d="M4 20 L12 20 L15 12 L18 28 L21 15 L24 23 L27 20 L33 20"
        />
        <path className="hud-nosignal__flat" d="M37 20 L44 20 M50 20 L60 20" />
        <line className="hud-nosignal__sweep" x1="32" y1="4" x2="32" y2="36" />
      </svg>
      <div className="hud-nosignal__text">
        <span className="hud-nosignal__title">{title}</span>
        {resolvedHint ? <span className="hud-nosignal__hint">{resolvedHint}</span> : null}
      </div>
    </div>
  );
}
