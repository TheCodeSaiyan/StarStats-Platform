/**
 * PLAIN MODULE, NO `'use client'`. The page is a server component and reads
 * `.key` off these to bucket its sections; every export of a client module
 * reaches the server as a client reference rather than the value, which on the
 * Emitter port produced a surface with a correct lens rail and no panes at all.
 */
import type { SurfaceGroup } from '@/components/projection/PaneSurface';

export const DIRECTORY_GROUP: SurfaceGroup = {
  key: 'directory',
  label: 'Directory',
};

export const DISCOVER_GROUPS: readonly SurfaceGroup[] = [DIRECTORY_GROUP];
