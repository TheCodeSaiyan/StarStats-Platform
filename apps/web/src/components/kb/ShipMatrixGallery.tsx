'use client';

/**
 * Ship Matrix image gallery with an expand-to-lightbox view.
 *
 * The thumbnail grid mirrors the old static layout, but each tile is now
 * a button that opens a full-screen lightbox (portaled to `document.body`
 * so no `transform`/`overflow` ancestor traps the fixed overlay). The
 * lightbox supports keyboard nav (Esc to close, ←/→ to page), a counter,
 * prev/next controls, and click-scrim-to-close.
 *
 * Client component (hover + open state). `mediaUrls` are the same
 * index-stable proxied URLs the server built — the proxy serves the full
 * image, so the lightbox reuses them at native size.
 */

import React, { useCallback, useEffect, useState } from 'react';
import { createPortal } from 'react-dom';

export function ShipMatrixGallery({ mediaUrls }: { mediaUrls: string[] }) {
  const [open, setOpen] = useState<number | null>(null);
  const count = mediaUrls.length;

  const close = useCallback(() => setOpen(null), []);
  const step = useCallback(
    (delta: number) =>
      setOpen((i) => (i === null ? i : (i + delta + count) % count)),
    [count],
  );

  // Keyboard control + scroll lock while the lightbox is open.
  useEffect(() => {
    if (open === null) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') close();
      else if (e.key === 'ArrowRight') step(1);
      else if (e.key === 'ArrowLeft') step(-1);
    };
    window.addEventListener('keydown', onKey);
    const prevOverflow = document.body.style.overflow;
    document.body.style.overflow = 'hidden';
    return () => {
      window.removeEventListener('keydown', onKey);
      document.body.style.overflow = prevOverflow;
    };
  }, [open, close, step]);

  if (count === 0) return null;

  return (
    <>
      <div
        data-testid="ship-matrix-gallery"
        style={{
          marginTop: 14,
          display: 'grid',
          gridTemplateColumns: 'repeat(auto-fill, minmax(160px, 1fr))',
          gap: 10,
        }}
      >
        {mediaUrls.map((url, idx) => (
          <button
            key={url}
            type="button"
            onClick={() => setOpen(idx)}
            aria-label={`Expand Ship Matrix image ${idx + 1}`}
            className="ss-gallery-thumb"
            style={{
              position: 'relative',
              padding: 0,
              border: '1px solid var(--border, rgba(255,255,255,0.07))',
              borderRadius: 6,
              overflow: 'hidden',
              cursor: 'zoom-in',
              background: 'var(--bg-elev, rgba(255,255,255,0.03))',
              display: 'block',
              width: '100%',
            }}
          >
            <img
              src={url}
              alt={`Ship Matrix image ${idx + 1}`}
              loading="lazy"
              style={{
                width: '100%',
                height: 'auto',
                display: 'block',
                objectFit: 'cover',
                aspectRatio: '16 / 9',
              }}
            />
            {/* Expand affordance — corner glyph, brightens on hover. */}
            <span
              aria-hidden
              style={{
                position: 'absolute',
                right: 6,
                bottom: 6,
                width: 22,
                height: 22,
                display: 'grid',
                placeItems: 'center',
                borderRadius: 4,
                fontSize: 12,
                color: 'var(--fg)',
                background: 'rgba(0,0,0,0.55)',
                border: '1px solid rgba(255,255,255,0.18)',
              }}
            >
              ⤢
            </span>
          </button>
        ))}
      </div>

      {open !== null &&
        createPortal(
          <div
            role="dialog"
            aria-modal="true"
            aria-label={`Ship Matrix image ${open + 1} of ${count}`}
            onClick={close}
            style={{
              position: 'fixed',
              inset: 0,
              zIndex: 1000,
              background: 'rgba(8,6,12,0.9)',
              backdropFilter: 'blur(4px)',
              display: 'grid',
              gridTemplateRows: 'auto 1fr auto',
              gap: 8,
              padding: '14px 14px 20px',
            }}
          >
            {/* Top bar: counter + close. */}
            <div
              style={{
                display: 'flex',
                alignItems: 'center',
                justifyContent: 'space-between',
              }}
            >
              <span style={{ fontSize: 12, color: 'var(--fg-muted)', fontFamily: 'var(--font-mono)' }}>
                {open + 1} / {count}
              </span>
              <button
                type="button"
                onClick={(e) => { e.stopPropagation(); close(); }}
                aria-label="Close"
                style={lightboxBtn}
              >
                ✕
              </button>
            </div>

            {/* Image (clicking it doesn't close — only the scrim does). */}
            <div style={{ display: 'grid', placeItems: 'center', minHeight: 0 }}>
              {/* eslint-disable-next-line @next/next/no-img-element */}
              <img
                src={mediaUrls[open]}
                alt={`Ship Matrix image ${open + 1}`}
                onClick={(e) => e.stopPropagation()}
                style={{
                  maxWidth: '92vw',
                  maxHeight: '78vh',
                  objectFit: 'contain',
                  borderRadius: 8,
                  boxShadow: '0 12px 48px rgba(0,0,0,0.6)',
                }}
              />
            </div>

            {/* Prev / next (hidden for a single image). */}
            {count > 1 && (
              <div style={{ display: 'flex', justifyContent: 'center', gap: 12 }}>
                <button
                  type="button"
                  onClick={(e) => { e.stopPropagation(); step(-1); }}
                  aria-label="Previous image"
                  style={lightboxBtn}
                >
                  ‹ Prev
                </button>
                <button
                  type="button"
                  onClick={(e) => { e.stopPropagation(); step(1); }}
                  aria-label="Next image"
                  style={lightboxBtn}
                >
                  Next ›
                </button>
              </div>
            )}
          </div>,
          document.body,
        )}
    </>
  );
}

const lightboxBtn: React.CSSProperties = {
  fontSize: 13,
  padding: '6px 12px',
  borderRadius: 6,
  cursor: 'pointer',
  color: 'var(--fg)',
  background: 'rgba(255,255,255,0.08)',
  border: '1px solid rgba(255,255,255,0.16)',
};
