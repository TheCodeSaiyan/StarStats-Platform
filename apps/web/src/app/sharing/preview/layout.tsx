import React from 'react';
import { AppSectionSurface } from '@/components/projection/AppSectionSurface';

/**
 * Projection frame for the `sharing/preview` segment.
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
        { label: 'Sharing', href: '/sharing' },
        { label: 'Preview' },
      ]}
      title="Share preview"
      ctx="What a recipient will see"
    >
      {children}
    </AppSectionSurface>
  );
}
