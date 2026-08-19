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

interface ErrorBoundaryProps {
  error: Error & { digest?: string };
  reset: () => void;
}

export default function GlobalError({ error, reset }: ErrorBoundaryProps) {
  useEffect(() => {
    console.error('StarStats render error:', error);
  }, [error]);

  return (
    <main>
      <h1>Something went wrong</h1>
      <div className="ss-alert ss-alert--danger" role="alert">
        {error.message || 'An unexpected error occurred.'}
      </div>
      <p className="muted">
        The page failed to render. You can try again, or head back home.
      </p>
      <p style={{ display: 'flex', gap: 12, alignItems: 'center' }}>
        <button
          type="button"
          className="ss-btn ss-btn--primary"
          onClick={() => reset()}
        >
          Try again
        </button>
        <Link href="/" className="ss-btn ss-btn--ghost">
          Back to home
        </Link>
      </p>
    </main>
  );
}
