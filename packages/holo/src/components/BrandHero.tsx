'use client';

import React from 'react';

/**
 * The signed-out hero: the brand itself, projected at the centre of the
 * volume where a lens would otherwise put a figure.
 *
 * On the landing page the core readout is the wrong instrument — a visitor who
 * has never heard of StarStats needs the name and the promise before a number
 * means anything. Same glow, same fringe, same position as `CoreReadout`, so
 * the ring still frames it.
 *
 * Exactly one per page; a second brand statement reads as a template. Never on
 * a signed-in screen — there the centre belongs to the reader's data.
 *
 * GEOMETRY: the enclosing `Projection` must be `surface="brand"`. That is what
 * opens the ring to `min(760px, 72vw)`; without it the hero overflows it. The
 * wordmark is sized FROM the ring, so the lockup always sits inside the circle
 * and clear of the segment labels.
 *
 * ONE ADDITION over the kit's version: `prefers-reduced-motion` pins the
 * rotation to the first word. The flat product's `HeroRotator` does exactly
 * this and says why — the CSS sweep is already flattened by the global
 * reduced-motion rule, but the CONTENT SWAP itself still registers as motion to
 * a screen reader or a refresh-driven UI, so pinning is what kills both
 * signals. Porting the hero without it would have been a quiet accessibility
 * regression.
 */
export interface BrandHeroProps {
  name?: string;
  tagline?: string;
  /** Cycles after the tagline — the thing being tracked. */
  words?: readonly string[];
  intervalMs?: number;
  detail?: React.ReactNode;
}

export function BrandHero({
  name = 'Starstats',
  tagline = 'Track your Star Citizen play.',
  words = [],
  intervalMs = 2400,
  detail,
}: BrandHeroProps) {
  const [i, setI] = React.useState(0);
  const [reduced, setReduced] = React.useState(false);

  React.useEffect(() => {
    if (typeof window === 'undefined' || !window.matchMedia) return;
    const mq = window.matchMedia('(prefers-reduced-motion: reduce)');
    const apply = () => setReduced(mq.matches);
    apply();
    mq.addEventListener('change', apply);
    return () => mq.removeEventListener('change', apply);
  }, []);

  React.useEffect(() => {
    if (reduced || words.length < 2) return;
    const t = setInterval(
      () => setI((n) => (n + 1) % words.length),
      intervalMs,
    );
    return () => clearInterval(t);
  }, [reduced, words.length, intervalMs]);

  return (
    <div className="hp-brand-hero">
      <div className="nm" data-v={name.toUpperCase()}>
        {name.toUpperCase()}
      </div>
      {tagline ? <div className="tg">{tagline}</div> : null}
      {words.length ? (
        <div className="wd" key={i}>
          {words[i]}
        </div>
      ) : null}
      {detail ? <div className="dt">{detail}</div> : null}
    </div>
  );
}
