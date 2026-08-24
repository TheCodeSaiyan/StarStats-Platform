'use client';

/**
 * The link component the projection's ranked rows are rendered as.
 *
 * `MeterRow` takes a component rather than a render callback because the
 * planes are assembled in `elements.tsx`, which is a SERVER module: a function
 * prop cannot cross the RSC boundary, a client-component reference can. This
 * file is that reference, and it exists only to hold the two Next-specific
 * details `holo` must not know about — the `Route` type and `prefetch`.
 *
 * `prefetch={false}` for the same reason `EntityLink` sets it: a projection
 * puts dozens of KB links on screen at once, and prefetching each one runs a
 * full KB detail render against the per-IP rate-limited reference API. Doing
 * that on viewport entry trips the governor and 429s the page the reader then
 * clicks. The KB routes have loading skeletons, so navigation still feels
 * immediate.
 */

import React from 'react';
import Link from 'next/link';
import type { Route } from 'next';

export function RowLink({
  href,
  className,
  children,
}: {
  href: string;
  className?: string;
  children?: React.ReactNode;
}) {
  return (
    <Link href={href as Route} className={className} prefetch={false}>
      {children}
    </Link>
  );
}
