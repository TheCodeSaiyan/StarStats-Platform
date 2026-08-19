'use client';

import { useCallback } from 'react';

/**
 * Mobile-only backdrop. Visible whenever `body[data-drawer="open"]`.
 * Clicking it dismisses the drawer.
 */
export function DrawerScrim() {
  const onClick = useCallback(() => {
    delete document.body.dataset.drawer;
  }, []);
  return <div className="ss-drawer-scrim" onClick={onClick} aria-hidden="true" />;
}
