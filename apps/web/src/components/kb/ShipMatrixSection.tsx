import React from 'react';
import { Plane, HoloKV } from 'holo';
import {
  shipMatrixSpecRows,
  type ShipMatrix,
} from '@/lib/ship-matrix';
import { ShipMatrixDisclaimer } from './ShipMatrixDisclaimer';
import { ShipMatrixGallery } from './ShipMatrixGallery';

interface ShipMatrixSectionProps {
  /** Validated `metadata.ship_matrix` blob (parsed at the boundary by
   *  the page — see `parseShipMatrix`). */
  shipMatrix: ShipMatrix;
  /** Pre-built, proxied media URLs (one per `media[]` index, pointing
   *  at `/v1/reference/vehicles/{class_name}/media/{idx}`). Built
   *  server-side because the API base is server-only. Pass an EMPTY
   *  array to suppress the gallery entirely — the backend media
   *  kill-switch can serve images dark, so the caller never assumes
   *  they load. */
  mediaUrls: string[];
}

/**
 * Vehicle-only Ship Matrix enrichment section: a structured specs
 * grid, the description block, an optional proxied-image gallery, and
 * the mandatory CIG disclaimer.
 *
 * Presentational only — all validation happens upstream in
 * `parseShipMatrix`, and media URLs are pre-built by the server
 * component (the API base is server-only). The component renders
 * nothing for the gallery when `mediaUrls` is empty, so it degrades
 * gracefully when images are absent or killed by the backend switch.
 *
 * The caller is responsible for only rendering this for
 * `category === 'vehicle'` when `metadata.ship_matrix` parsed
 * successfully.
 *
 * REDRAWN. It was an `.ss-card` with a hand-built `<dl>` grid and five inline
 * type sizes. It is a `Plane` with a `HoloKV` now — same rows, same order, same
 * copy, same disclaimer. The `heading` prop still suppresses the caption when a
 * pane header above already carries the words.
 */
export function ShipMatrixSection({
  shipMatrix,
  mediaUrls,
  heading = true,
}: ShipMatrixSectionProps & {
  /**
   * Render the component's own "Ship Matrix" `<h2>`.
   *
   * The projection frames each section in a `Pane` whose header IS an `<h2>`
   * carrying the section name, so inside one this heading would announce
   * "Ship Matrix" twice — once as the pane header, once nested. Defaults to
   * true so the flat page and this component's own tests are unchanged; the
   * projection opts out and puts the same words, verbatim, in the pane title.
   */
  heading?: boolean;
}) {
  const specRows = shipMatrixSpecRows(shipMatrix);
  const hasGallery = mediaUrls.length > 0;

  return (
    <Plane
      tilt="flat"
      cap={heading ? 'Ship Matrix' : undefined}
      hint="official specifications"
      aria-label="Ship Matrix"
      style={{ marginTop: 16 }}
    >
      {/* CIG's own copy about CIG's own data — kept verbatim, and kept as the
          plane's own line rather than the caption hint so it reads as a source
          note rather than a label. */}
      <p className="hp-prose">Official specifications from RSI&apos;s Ship Matrix.</p>

      {specRows.length > 0 ? (
        <HoloKV
          items={specRows.map((row) => ({ k: row.label, v: row.value }))}
        />
      ) : null}

      {shipMatrix.description ? (
        <p className="hp-prose hp-prose--pre" style={{ marginTop: 14 }}>
          {shipMatrix.description}
        </p>
      ) : null}

      {hasGallery && <ShipMatrixGallery mediaUrls={mediaUrls} />}

      <ShipMatrixDisclaimer />
    </Plane>
  );
}
