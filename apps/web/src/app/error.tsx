'use client';

/**
 * Root error boundary. Next.js renders this when a route segment throws
 * during render or in a Server Action. It must be a Client Component
 * because it owns the `reset()` recovery handler.
 *
 * `console.error` is the conventional fallback log for an error boundary
 * — there is no server logger available here, and we still want the
 * trace surfaced in the browser console for debugging.
 */

import Link from 'next/link';
import { useEffect } from 'react';
import { BeamButton, BeamAlert } from 'holo';
import { BoundaryShell } from '@/components/projection/BoundaryShell';

interface ErrorBoundaryProps {
  error: Error & { digest?: string };
  reset: () => void;
}

export default function GlobalError({ error, reset }: ErrorBoundaryProps) {
  useEffect(() => {
    console.error('StarStats render error:', error);
  }, [error]);

  // The crumb carries the `<h1>`, so the heading is not repeated in the body —
  // same rule every other projection surface follows. Copy is unchanged.
  return (
    <BoundaryShell crumb="Something went wrong">
      <BeamAlert tone="bad">
        {error.message || 'An unexpected error occurred.'}
      </BeamAlert>
      <p className="hp-prose">
        The page failed to render. You can try again, or head back home.
      </p>
      <div className="hp-formrow">
        <BeamButton type="button" variant="primary" onClick={() => reset()}>
          Try again
        </BeamButton>
        <Link href="/" className="hp-btn hp-btn--ghost">
          Back to home
        </Link>
      </div>
    </BoundaryShell>
  );
}
