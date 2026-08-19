'use client';

import { useCallback } from 'react';

/**
 * Mobile-only hamburger that flips `body[data-drawer]` between
 * `open` and unset. The CSS in starstats-tokens.css drives the
 * left-rail slide-in and scrim opacity from that attribute.
 */
export function DrawerToggle() {
  const onClick = useCallback(() => {
    if (document.body.dataset.drawer === 'open') {
      delete document.body.dataset.drawer;
    } else {
      document.body.dataset.drawer = 'open';
    }
  }, []);

  return (
    <button
      type="button"
      className="ss-drawer-toggle"
      onClick={onClick}
      aria-label="Toggle navigation"
    >
      <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round">
        <path d="M2 4h12M2 8h12M2 12h12" />
      </svg>
    </button>
  );
}
