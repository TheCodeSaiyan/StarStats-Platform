import React from 'react';
import { AppSectionSurface } from '@/components/projection/AppSectionSurface';

/**
 * Projection frame for `/settings/widget-sharing`.
 *
 * THE LAST ROUTE IN THE PRODUCT TO PORT. With this one framed, no page renders
 * the flat `TopBar` or `LeftRail` any more — `projection-shell.css` hid them
 * wherever a projection was present, and there is nowhere left that isn't.
 *
 * Framed from a layout rather than the page for consistency with the other
 * `/settings` sub-route work and because it costs nothing: the page keeps its
 * own body and its own `<h1>`.
 */
export default function WidgetSharingLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return (
    <AppSectionSurface
      crumb={[
        { label: 'Projection', href: '/me' },
        { label: 'Calibrate', href: '/settings' },
        { label: 'Widget sharing' },
      ]}
      title="Widget sharing"
      ctx="Per-widget visibility on your public profile"
    >
      {children}
    </AppSectionSurface>
  );
}
