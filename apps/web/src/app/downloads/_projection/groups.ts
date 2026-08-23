/**
 * The Emitter's lens groups.
 *
 * PLAIN MODULE, NO `'use client'`, DELIBERATELY. These constants started out in
 * `EmitterProjection.tsx` next to the component that consumes them, and the
 * page rendered completely empty — a lens rail with three correctly-labelled
 * groups and not one pane under any of them.
 *
 * The cause is worth writing down because nothing reports it. Every export of a
 * `'use client'` module becomes a CLIENT REFERENCE when a server component
 * imports it, not the value itself. So `EMITTER_GROUP.key` read on the server
 * was `undefined`, every section was built with `group: undefined`, and nothing
 * ever matched a group key. Meanwhile the same objects passed as a prop were
 * resolved back to real objects on the client, so the rail's labels rendered
 * perfectly — which is exactly why it looked like a filtering bug rather than
 * an import one. No type error, no console error, no failed request.
 *
 * Anything a server component reads the CONTENTS of must live outside the
 * client boundary.
 */
import type { SurfaceGroup } from '@/components/projection/PaneSurface';

export const EMITTER_GROUP: SurfaceGroup = { key: 'get', label: 'Get it' };
export const PAIR_GROUP: SurfaceGroup = { key: 'pair', label: 'Pair' };
export const UPLINKS_GROUP: SurfaceGroup = { key: 'uplinks', label: 'Uplinks' };
