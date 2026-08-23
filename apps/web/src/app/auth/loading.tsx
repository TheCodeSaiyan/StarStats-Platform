import React from 'react';

/**
 * Auth-section loading skeleton — eyebrow, heading, two fields, a CTA.
 *
 * NOT a `<main>`. The projection puts `role="main"` on `#hp-content`, so the
 * element this used to be added a SECOND main landmark for as long as a page
 * was loading, and picked up globals.css's legacy 720px `main` column while it
 * was there. A div with the section's own classes has neither problem.
 */
export default function AuthLoading() {
  return (
    <div className="hp-authpage" aria-busy="true" aria-label="Loading">
      <div className="hp-authcard">
        <div className="skeleton" style={{ height: 12, width: 80 }} />
        <div className="skeleton" style={{ height: 30, width: 220 }} />
        <div className="skeleton" style={{ height: 44, width: '100%' }} />
        <div className="skeleton" style={{ height: 44, width: '100%' }} />
        <div className="skeleton" style={{ height: 40, width: 120 }} />
      </div>
    </div>
  );
}
