'use client';

import React from 'react';
import { Projection, Crumb } from 'holo';

/**
 * Projection frame for the error and not-found boundaries.
 *
 * WHY THESE NEEDED ONE AT ALL. A root `error.tsx` replaces everything below the
 * ROOT layout, not the root layout itself — so these two rendered inside
 * `layout.tsx`'s `.ss-app` with no `.ss-projection-root` anywhere, which is
 * precisely the condition `projection-shell.css` keys on to hide the flat
 * chrome. Result: every route in the product was a projection, and the two
 * screens a reader sees when something has gone wrong were still wearing the
 * old shell. It showed up while probing a route that looked unported and was
 * actually just throwing.
 *
 * NO `ChromeBar`, deliberately. It would need a session and a nav, and
 * `error.tsx` is a Client Component that cannot read cookies — so the chrome
 * would either be a lie (signed-out furniture shown to a signed-in reader) or
 * a second data dependency on the screen that exists because a data dependency
 * failed. The crumb and the body's own actions are enough to leave from.
 *
 * `surface="console"` kills the ambience: an error is not the moment for a
 * parallax stage, and the reduced-motion argument applies doubly to someone who
 * has just been interrupted.
 */
export function BoundaryShell({
  crumb,
  children,
}: {
  crumb: string;
  children: React.ReactNode;
}) {
  return (
    <div className="ss-projection-root">
      <Projection
        surface="console"
        parallax={false}
        crumb={<Crumb heading parts={[{ t: crumb }]} />}
      >
        <div className="hp-settings">
          <div className="hp-settings__inner hp-boundary">{children}</div>
        </div>
      </Projection>
    </div>
  );
}
