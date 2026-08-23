import React from 'react';
import type { StatRow } from '@/lib/kb-viz';

/**
 * One peer-relative stat: the figure, a min→max track carrying the value and
 * the class median, and the quantile band that says what the position means.
 *
 * REDRAWN. The flat version was a rounded pill with a hardcoded amber gradient
 * (`#E8A23C`), a 4px radius and a round value dot — three things the system
 * does not have. Worse, the colour was literal rather than tokenised, so it
 * stayed amber when the reader recalibrated to Pyro or Nyx: the one element on
 * the page whose whole job is comparison did not belong to the beam.
 *
 * It is a hairline track now. Height and brightness carry the value; hue never
 * does. The median is a tick, not a second colour. The band keeps its tone
 * through the system's own classes rather than `TONE_COLOR`.
 *
 * Degrades to label + figure when the row has no `fillPct` — a stat with no
 * peer distribution has no position to show, and an empty track would imply
 * one.
 */
export function StatBar({ row }: { row: StatRow }) {
  const hasTrack = row.fillPct !== undefined;
  return (
    <div className="hp-statbar">
      <div className="hp-statbar__head">
        <span className="hp-statbar__label">{row.label}</span>
        <b className="hp-statbar__value">{row.valueText}</b>
      </div>
      {hasTrack ? (
        <div
          className="hp-statbar__track"
          role="img"
          aria-label={`${row.label}: ${row.valueText}${
            row.band ? `, ${row.band.text}` : ''
          }`}
        >
          <i className="fill" style={{ width: `${row.fillPct}%` }} />
          {row.medianPct !== undefined ? (
            <i
              className="median"
              style={{ left: `${row.medianPct}%` }}
              title="class median"
            />
          ) : null}
        </div>
      ) : null}
      {row.band ? (
        <span className={`hp-statbar__band ${row.band.tone}`}>
          {row.band.text}
        </span>
      ) : null}
    </div>
  );
}
