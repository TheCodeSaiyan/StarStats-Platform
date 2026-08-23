import React from 'react';
import { AppSectionSurface } from '@/components/projection/AppSectionSurface';

/**
 * Projection frame for the `submissions/new` segment.
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
        { label: 'Projection', href: '/me' },
        { label: 'Submissions', href: '/submissions' },
        { label: 'New' },
      ]}
      title="New submission"
      ctx="Propose a parser rule"
    >
      {children}
    </AppSectionSurface>
  );
}
