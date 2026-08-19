import React from 'react';
import { SHIP_MATRIX_DISCLAIMER } from 'reference-data/attribution';

/**
 * Required CIG attribution / disclaimer shown on every surface that
 * renders RSI Ship Matrix data (specs, description, images).
 *
 * The wording is fixed by the design spec (brand-pack §11 style) and
 * is mitigation for the copyrighted-expression portion of the
 * enrichment (description prose + official images); it must appear
 * wherever the Ship Matrix section renders. Keep the body text
 * verbatim — do not paraphrase. The exact string is centralised in the
 * `reference-data` package's attribution module (the single source of
 * truth for reference-data credits); JSX collapses inter-word
 * whitespace, so a single-string constant renders byte-identically to
 * the previous multi-line JSX text node.
 *
 * Pure presentational component (no props, no state) so it can be
 * dropped into the server-rendered vehicle KB page and unit-tested in
 * isolation. Styled inline with the same CSS variables the rest of the
 * KB detail page uses, so it reads as a muted footnote under the data.
 */
export function ShipMatrixDisclaimer() {
  return (
    <aside
      aria-label="Ship Matrix attribution"
      style={{
        marginTop: 14,
        paddingTop: 12,
        borderTop: '1px solid var(--border, rgba(255,255,255,0.08))',
        fontSize: 11,
        lineHeight: 1.5,
        color: 'var(--fg-dim)',
      }}
    >
      {SHIP_MATRIX_DISCLAIMER}
    </aside>
  );
}
