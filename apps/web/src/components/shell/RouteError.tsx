'use client';

import Link from 'next/link';
import type { Route } from 'next';
import { useEffect } from 'react';

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

  return (
    <main>
      <h1>{title}</h1>
      <p className="muted" style={{ maxWidth: '52ch' }}>
        This page failed to load. You can try again, or head back and try
        another way in.
      </p>
      <p style={{ display: 'flex', gap: 12, alignItems: 'center' }}>
        <button
          type="button"
          className="ss-btn ss-btn--primary"
          onClick={() => reset()}
        >
          Try again
        </button>
        <Link href={backHref as Route} className="ss-btn ss-btn--ghost">
          {backLabel}
        </Link>
      </p>
    </main>
  );
}
