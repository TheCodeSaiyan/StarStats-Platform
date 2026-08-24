import React from 'react';

/**
 * The projection's headline figure: split into a chromatic fringe, unit set
 * small and thin, with a tracked caption and a detail line beneath.
 *
 * `value` is echoed by the ::before/::after fringe layers via `data-v`, so
 * keep it SHORT — four glyphs or fewer reads best. Never put two core
 * readouts in one volume; a second figure belongs in a Callout or SubStats.
 *
 * On /me this carries lifetime playtime (decisions A5 + B4): the one
 * range-INDEPENDENT anchor in a volume whose callouts and ring are all
 * scoped to the selected range.
 */
export interface CoreReadoutProps {
  /** Short. Echoed into `data-v` for the chromatic fringe layers. */
  value: string;
  unit?: React.ReactNode;
  label?: React.ReactNode;
  detail?: React.ReactNode;
}

export function CoreReadout({ value, unit, label, detail }: CoreReadoutProps) {
  return (
    <div className="hp-core">
      <div className="n">
        {/* THE FRINGE BELONGS TO THE VALUE, NOT THE LINE.
            `data-v` was on this row while the row also holds the unit, and
            the fringe pseudo-elements were stretched across it (`left` AND
            `right` set). So the ghost text — the value alone — centred over
            value+unit while the real value sits to the LEFT of the unit, and
            the two drifted apart by roughly half the unit's width. Invisible
            on a short unit, glaring on a word: Travel's "44 jumps" threw the
            ghost right off the figure while the other lenses looked fine.
            Its own span means the ghost box is the value's box at any unit
            width. */}
        <span className="v" data-v={value}>
          {value}
        </span>
        {unit ? <em>{unit}</em> : null}
      </div>
      {label ? <div className="u">{label}</div> : null}
      {detail ? <div className="d">{detail}</div> : null}
    </div>
  );
}
