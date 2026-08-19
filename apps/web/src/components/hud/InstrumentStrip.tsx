import React from 'react';

export interface Readout {
  k: string;
  v: React.ReactNode;
}

export function InstrumentStrip({
  title,
  context,
  readouts = [],
  trailing,
  size = 'default',
}: {
  title: React.ReactNode;
  context?: React.ReactNode;
  readouts?: Readout[];
  trailing?: React.ReactNode;
  /**
   * `'default'` (the compact 14px instrument-strip title used by
   * dashboards/list pages) or `'hero'` (a larger, clamped title for
   * detail/hero pages — KB entity detail, public profile). Hero callers
   * should give their title `<h1>` `fontSize: 'inherit'` so it picks up
   * this wrapper size instead of its own `.hud-tile__title` 12px.
   */
  size?: 'default' | 'hero';
}) {
  return (
    <header className="hud-strip">
      <div
        className="hud-tile__title"
        style={{ fontSize: size === 'hero' ? 'clamp(22px, 3vw, 28px)' : 14 }}
      >
        {title}
      </div>
      {context ? (
        <span className="hud-tile__sub" style={{ marginLeft: 0 }}>
          {context}
        </span>
      ) : null}
      <span style={{ flex: 1 }} />
      {readouts.map((r, i) => (
        <span key={i} className="hud-readout">
          <span className="k">{r.k}</span>
          {r.v}
        </span>
      ))}
      {trailing}
    </header>
  );
}
