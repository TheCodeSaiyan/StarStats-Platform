'use client';

import Link from 'next/link';
import type { Route } from 'next';
import { useEffect } from 'react';
import { BeamButton } from 'holo';
import { BoundaryShell } from '@/components/projection/BoundaryShell';

interface RouteErrorProps {
  error: Error & { digest?: string };
  reset: () => void;
  /** Page-specific heading, e.g. "Couldn't load your timeline". */
  title?: string;
  /** Where the "back" affordance points. Defaults to the dashboard. */
  backHref?: string;
  backLabel?: string;
}

/**
 * Shared segment-level error boundary UI. Per-segment `error.tsx` files
 * render this so a render throw on one page (timeline, sharing, kb…)
 * shows an in-place recovery card instead of bubbling to the root
 * boundary and replacing the whole shell. Client Component — it owns
 * the `reset()` recovery handler. The detailed error is logged to the
 * console only; the user sees a clean, generic message (raw `reason`
 * strings can leak internals — see the UI/UX audit).
 *
 * IT CARRIES ITS OWN PROJECTION FRAME. A segment `error.tsx` replaces the
 * segment's PAGE, so on the routes that use this (dashboard, journey, kb,
 * sharing) there is no `.ss-projection-root` left in the tree — and that is the
 * exact condition `projection-shell.css` keys on to hide the flat chrome. Every
 * route in the product is a projection; without this, the screens shown when
 * one of them fails were not.
 *
 * CAVEAT for a future `error.tsx`: a segment already framed by a projection
 * `layout.tsx` (`/auth`, `/admin`, `/submissions/[id]`, …) keeps that frame
 * when its page throws, so an `error.tsx` there must NOT use this component —
 * it would nest a second volume inside the first. Render the body alone.
 */
export function RouteError({
  error,
  reset,
  title = 'Something went wrong',
  backHref = '/me',
  backLabel = 'Back to overview',
}: RouteErrorProps) {
  useEffect(() => {
    console.error('StarStats route error:', error);
  }, [error]);

  // The crumb carries the `<h1>`; copy is otherwise unchanged, including the
  // deliberately generic body — a raw `reason` can leak internals.
  return (
    <BoundaryShell crumb={title}>
      <p className="hp-prose">
        This page failed to load. You can try again, or head back and try
        another way in.
      </p>
      <div className="hp-formrow">
        <BeamButton type="button" variant="primary" onClick={() => reset()}>
          Try again
        </BeamButton>
        <Link href={backHref as Route} className="hp-btn hp-btn--ghost">
          {backLabel}
        </Link>
      </div>
    </BoundaryShell>
  );
}
