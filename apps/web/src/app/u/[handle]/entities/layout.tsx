import React from 'react';
import { AppSectionSurface } from '@/components/projection/AppSectionSurface';

/**
 * Projection frame for the `u/[handle]/entities` segment.
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
        { label: 'Entities' },
      ]}
      title="Entities"
      ctx="Ships, weapons and places, as seen"
    >
      {children}
    </AppSectionSurface>
  );
}
