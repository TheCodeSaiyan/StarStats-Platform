'use client';

import React from 'react';
import { useRouter } from 'next/navigation';
import { LayoutEditor, useLayout } from 'holo';
import { PROFILE_CATALOGUE } from './catalogue';

/**
 * The owner's arrange mode for `/u/[handle]`.
 *
 * Replaces the flat `WidgetCanvas` — a 24-column drag grid inherited from
 * before the port — with the projection's own layout editor, the same one
 * `/me` uses. That is not only consistency: the grid was editing coordinates
 * this surface stopped reading when the body became a plane stack, so an owner
 * could drag a tile into a position that then rendered nowhere. What the dock
 * can express is WHICH elements appear and in WHAT ORDER, and that is exactly
 * what `useLayout` persists.
 *
 * `initial` comes from the server-rendered layout rather than the catalogue's
 * defaults, so the editor opens on what the account actually holds. A save is
 * followed by `router.refresh()` for the reason `MeProjection` records: the
 * server built the dock from the OLD layout, so without a re-render a
 * newly-enabled element is listed as on and draws nothing. The refresh is
 * fired after the write, never raced with it.
 */
export function ArrangeProfile({
  handle,
  enabledIds,
  onSave,
}: {
  handle: string;
  enabledIds: string[];
  onSave: (ids: string[]) => Promise<{ ok: boolean }>;
}) {
  const router = useRouter();

  const persist = React.useCallback(
    async (ids: string[]) => {
      const res = await onSave(ids);
      // A refused write is not a saved one: throwing puts the editor in its
      // error state, while the refresh still runs so what is on screen — and,
      // through `initial`, the editor's own list — returns to what the server
      // holds rather than showing a change that was never stored.
      router.refresh();
      if (!res?.ok) throw new Error('layout_save_refused');
    },
    [onSave, router],
  );

  const layout = useLayout(`profile:${handle}`, PROFILE_CATALOGUE, {
    initial: enabledIds,
    persist,
  });

  return (
    <LayoutEditor
      catalogue={PROFILE_CATALOGUE}
      layout={layout}
      // DOCKED, not floating. The panel's default is `position: absolute;
      // top: 96px`, which anchors to the nearest positioned ancestor — in the
      // volume that is the stage, and it hangs beside the ring as `/me` shows
      // it. Here the nearest ancestor is `.hp-volume-below`, so the default
      // put it 96px below the top of the DOCK: off the bottom of a 900px
      // viewport, measured at y=996. Docked makes it static and it takes the
      // place of the body it is editing, which is also where the owner was
      // looking when they pressed Arrange.
      docked
      title="Profile layout"
      onClose={() => router.push(`/u/${encodeURIComponent(handle)}`)}
    />
  );
}
