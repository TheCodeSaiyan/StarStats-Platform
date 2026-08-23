import React from 'react';
import { AppSectionSurface } from '@/components/projection/AppSectionSurface';

/**
 * Projection frame for the `u/[handle]/sessions` segment.
 *
 * Framed from the layout rather than the page: these routes branch several
 * ways (not found, denied, empty, success) and each branch has its own
 * top-level return. See `AppSectionSurface` for the full reasoning.
 */
export default function SectionLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return (
    <AppSectionSurface
      crumb={[
        { label: 'Site', href: '/' },
        { label: 'Sessions' },
      ]}
      title="Sessions"
      ctx="One pilot's session log"
    >
      {children}
    </AppSectionSurface>
  );
}
