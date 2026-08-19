/**
 * Admin-segment loading skeleton.
 *
 * Creates a Suspense boundary at `/admin` so that navigating BETWEEN
 * admin pages (which share `admin/layout.tsx`) shows an instant
 * fallback instead of freezing on the previous page while the next
 * page's cold RSC resolves — the "admin pages are slow to change"
 * report (2026-07-20).
 *
 * This boundary only reaches the screen on a soft navigation if the
 * router has PREFETCHED the shell it lives in. That is why
 * `_components/AdminNav.tsx` must leave `prefetch` at its default: an
 * earlier `prefetch={false}` there stranded this file entirely, and
 * the resulting feedback-free wait was reported as "clicking the Audit
 * log tab does nothing". Note that Next.js prefetches in production
 * builds only, so this skeleton never appears under `next dev`.
 *
 * Deliberately a plain <div>, NOT <main>: `admin/layout.tsx` already
 * provides the sole `role="main"` landmark, and the global `main {}`
 * rule (globals.css) clamps to a 720px column — a nested <main> here
 * would duplicate the landmark and crush the full-width skeleton.
 */

// Explicit React import: this repo's vitest uses the classic JSX
// runtime, so a JSX-rendering component ReferenceErrors without it
// (the prod Next build uses the automatic runtime and doesn't need it).
import React from 'react';

export default function AdminLoading() {
  return (
    <div
      aria-busy="true"
      aria-label="Loading admin section"
      style={{ display: 'flex', flexDirection: 'column', gap: 20 }}
    >
      {/* Nav row placeholder — keeps the layout from jumping when the
          real page (which owns its own AdminNav) streams in. */}
      <div
        className="skeleton"
        style={{ height: 36, width: '100%', borderRadius: 'var(--r-pill)' }}
      />
      {/* Header block */}
      <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
        <div className="skeleton" style={{ height: 32, width: '40%' }} />
        <div className="skeleton" style={{ height: 16, width: '70%' }} />
      </div>
      {/* Content rows */}
      <div style={{ display: 'flex', flexDirection: 'column', gap: 10 }}>
        {[0, 1, 2, 3, 4].map((i) => (
          <div key={i} className="skeleton" style={{ height: 44, width: '100%' }} />
        ))}
      </div>
    </div>
  );
}
