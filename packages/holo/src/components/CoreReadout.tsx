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
      <div className="n" data-v={value}>
        {value}
        {unit ? <em>{unit}</em> : null}
      </div>
      {label ? <div className="u">{label}</div> : null}
      {detail ? <div className="d">{detail}</div> : null}
    </div>
  );
}
