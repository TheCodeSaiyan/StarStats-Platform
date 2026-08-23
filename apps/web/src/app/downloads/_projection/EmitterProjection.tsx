'use client';

import React from 'react';
import {
  PaneSurface,
  type SurfaceSection,
  type PaneSurfaceProps,
} from '@/components/projection/PaneSurface';

/**
 * `/downloads` — the **Emitter**, in the projection.
 *
 * This surface absorbed `/devices`. The design system names the desktop client
 * the emitter and treats its whole lifecycle as one thing: download it, pair it
 * to your account, watch what it sends, revoke it. `Downloads.jsx` lists "Pair
 * this machine to your account" as a step of installing, and COVERAGE records
 * the uplink table as belonging with the client rather than on a page of its
 * own. Splitting the two meant a reader who had just downloaded the tray had to
 * find a separate destination to make it do anything.
 *
 * It also freed the word **Hangar**, which was doing double duty: the nav
 * entry pointed at paired devices while the actual hangar — the RSI fleet — is
 * a widget and a pane elsewhere. `/devices` now redirects here, so every
 * inbound link (terms, the guides, signup, the fleet pane's refresh affordance)
 * still lands somewhere correct.
 *
 * PUBLIC. Anyone can read the download half signed out; the pairing and uplink
 * groups exist only when there is a session to pair to, and the page builds
 * only the groups it will render — a rail advertising "Uplinks" to a
 * signed-out visitor is exactly the label-leak the access model forbids.
 *
 * DEVICE TABS STAY LINKS, not rail state. Selecting a device re-fetches that
 * device's ingest batches server-side (`getIngestHistory` filtered by
 * `device_id`), so switching is a navigation, not a client toggle. Making the
 * rail own it would mean either shipping every device's batches to the client
 * or a client fetch — both worse than the `?device=` link that already works.
 */
export type EmitterSection = SurfaceSection;

// The group constants live in `./groups` — a plain module. A server component
// reads their `.key` to bucket its sections, and every export of a
// `'use client'` module reaches the server as a client reference rather than
// the value. See the comment there; it cost a completely blank surface.

export type EmitterProjectionProps = Omit<
  PaneSurfaceProps,
  'crumb' | 'account' | 'themeAction'
>;

export function EmitterProjection(props: EmitterProjectionProps) {
  return (
    <PaneSurface
      {...props}
      legal
      crumb={
        props.handle
          ? [{ label: 'Projection', href: '/me' }, { label: 'Emitter' }]
          : [{ label: 'Site', href: '/' }, { label: 'Emitter' }]
      }
      account={
        props.handle
          ? [
              { id: 'me', label: 'Projection', href: '/me' },
              { id: 'settings', label: 'Calibrate', href: '/settings' },
              { id: 'sharing', label: 'Sharing', href: '/sharing' },
            ]
          : undefined
      }
    />
  );
}
