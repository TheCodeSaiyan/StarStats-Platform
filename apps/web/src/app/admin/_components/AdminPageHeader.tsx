/**
 * Shared header for /admin pages: eyebrow, h1, optional lede.
 *
 * Every admin page used to hand-roll this block; this owns it for all 17.
 *
 * DRAWN BY THE SYSTEM NOW. The title was inline `32px / 600 / -0.02em` — the
 * flat voice, tight and semibold, where the beam voice is thin and positively
 * tracked — and inline meant no stylesheet could correct it. It takes
 * `.hp-pagetitle` instead, the same class every other page title in the app
 * uses, so an admin page reads like the rest of the product.
 *
 * The eyebrow keeps `ss-eyebrow`: it is one of the two sanctioned uses (a
 * section category label above a heading) and the bridge already redraws it
 * into the projection's tracked-caption idiom.
 */

// Explicit React import: this repo's vitest uses the classic JSX
// runtime, so a JSX-rendering component ReferenceErrors without it.
import React from 'react';

export function AdminPageHeader({
  eyebrow,
  title,
  lede,
  titleClassName,
  children,
}: {
  /** ReactNode so pages can interpolate a dynamic segment. */
  eyebrow: React.ReactNode;
  /** ReactNode, not string — several titles interpolate a value. */
  title: React.ReactNode;
  /** ReactNode, not string — ship-matrix and smtp ledes carry markup. */
  lede?: React.ReactNode;
  /** e.g. `mono` on the user-detail page, whose title is a handle. */
  titleClassName?: string;
  /** Controls that belong inside the header, below the lede. */
  children?: React.ReactNode;
}) {
  return (
    <header className="hp-adminhead">
      <div className="ss-eyebrow hp-adminhead__eyebrow">{eyebrow}</div>
      <h1
        className={
          titleClassName ? `hp-pagetitle ${titleClassName}` : 'hp-pagetitle'
        }
      >
        {title}
      </h1>
      {lede !== undefined && <p className="hp-adminhead__lede">{lede}</p>}
      {children}
    </header>
  );
}
