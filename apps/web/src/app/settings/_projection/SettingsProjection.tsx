'use client';

import React from 'react';
import {
  PaneSurface,
  type SurfaceSection,
  type SurfaceGroup,
  type PaneSurfaceProps,
} from '@/components/projection/PaneSurface';

/**
 * `/settings` — Calibrate, in the projection.
 *
 * The design kit's `Calibrate.jsx` was NOT read from this route (COVERAGE
 * marks it inferred), and it shows: it invents an Uplinks pairing table with a
 * pair code, a Retention section with storage figures and export formats, and
 * scanline / cloud-sync / public-projection switches. None of those exist on
 * `/settings` — pairing is `/devices`, visibility is `/sharing`, cloud sync is
 * a tray-side setting under the two-gate model, and retention/export has no
 * endpoint at all. So this ports the REAL route's ten sections and leaves the
 * kit's fiction out.
 *
 * The shell — chrome, rail, scrolling panes and the fragment wiring that keeps
 * `#security` / `#danger` / `#rsi` landing — is `PaneSurface`, shared with
 * `/sharing`.
 */
export type SettingsSection = SurfaceSection;
export type SettingsGroup = SurfaceGroup;

export type SettingsProjectionProps = Omit<PaneSurfaceProps, 'crumb' | 'account'>;

export function SettingsProjection(props: SettingsProjectionProps) {
  return (
    <PaneSurface
      {...props}
      crumb={[{ label: 'Projection', href: '/me' }, { label: 'Calibrate' }]}
      account={[
        { id: 'me', label: 'Projection', href: '/me' },
        { id: 'sharing', label: 'Sharing', href: '/sharing' },
        { id: 'downloads', label: 'Emitter', href: '/downloads' },
      ]}
    />
  );
}
