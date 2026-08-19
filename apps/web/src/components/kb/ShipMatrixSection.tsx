import React from 'react';
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
 */
export function ShipMatrixSection({
  shipMatrix,
  mediaUrls,
}: ShipMatrixSectionProps) {
  const specRows = shipMatrixSpecRows(shipMatrix);
  const hasGallery = mediaUrls.length > 0;

  return (
    <section
      className="ss-card"
      style={{ marginTop: 16, padding: '14px 16px' }}
      aria-label="Ship Matrix"
    >
      <h2
        style={{
          margin: '0 0 4px',
          fontSize: 14,
          fontWeight: 600,
          color: 'var(--fg)',
        }}
      >
        Ship Matrix
      </h2>
      <p
        style={{
          margin: '0 0 10px',
          fontSize: 11,
          color: 'var(--fg-muted)',
        }}
      >
        Official specifications from RSI&apos;s Ship Matrix.
      </p>

      {specRows.length > 0 && (
        <dl
          style={{
            display: 'grid',
            gridTemplateColumns: 'repeat(auto-fit, minmax(150px, 1fr))',
            gap: 12,
            margin: 0,
          }}
        >
          {specRows.map((row) => (
            <div key={row.label}>
              <dt
                style={{
                  color: 'var(--fg-muted)',
                  fontSize: 11,
                  textTransform: 'uppercase',
                  letterSpacing: '0.06em',
                }}
              >
                {row.label}
              </dt>
              <dd style={{ margin: '4px 0 0', fontSize: 14 }}>{row.value}</dd>
            </div>
          ))}
        </dl>
      )}

      {shipMatrix.description && (
        <p
          style={{
            margin: specRows.length > 0 ? '14px 0 0' : 0,
            fontSize: 13,
            lineHeight: 1.6,
            color: 'var(--fg)',
            whiteSpace: 'pre-line',
          }}
        >
          {shipMatrix.description}
        </p>
      )}

      {hasGallery && <ShipMatrixGallery mediaUrls={mediaUrls} />}

      <ShipMatrixDisclaimer />
    </section>
  );
}
