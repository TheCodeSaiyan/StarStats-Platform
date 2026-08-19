/**
 * Shared header for /admin pages: eyebrow, h1, optional lede.
 *
 * Every admin page used to hand-roll this block. The values here are
 * copied verbatim from those pages so the extraction is visually inert
 * — with one deliberate normalisation: pages varied between
 * `maxWidth: 640` and `720` on the lede, and this settles on 720.
 *
 * 32px is the sanctioned top-level opt-in above the 28px `main h1`
 * baseline; the mobile shrink in starstats-tokens.css applies because
 * this is a bare h1 (no `hud-tile__title` class).
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
    <header>
      <div className="ss-eyebrow" style={{ marginBottom: 8 }}>
        {eyebrow}
      </div>
      <h1
        className={titleClassName}
        style={{
          margin: 0,
          fontSize: 32,
          fontWeight: 600,
          letterSpacing: '-0.02em',
        }}
      >
        {title}
      </h1>
      {lede !== undefined && (
        <p
          style={{
            margin: '6px 0 0',
            color: 'var(--fg-muted)',
            fontSize: 14,
            maxWidth: 720,
            lineHeight: 1.55,
          }}
        >
          {lede}
        </p>
      )}
      {children}
    </header>
  );
}
