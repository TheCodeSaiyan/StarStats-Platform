import { LENSES } from '@/lib/lens';

/**
 * The lens rail for `/me`.
 *
 * Exported rather than inlined in `MeProjection` so it can be ASSERTED ON.
 * The first version of this was a local `LENSES.filter(l => l.id !== 'all')`,
 * which stranded every element whose `WIDGET_LENSES` entry is the empty list —
 * `records`, `orgs`, `entities` and `facts` — because All was the only lens any
 * of them matched. `facts` is enabled by default, so it rendered nowhere at
 * all, and no test could see the problem while the rail was a private const.
 *
 * It is the product's six lenses, unfiltered. Overview (no lens selected) stays
 * distinct from All: overview is the ring and the callouts, All is the pane
 * holding every enabled element.
 */
export const RAIL_LENSES = LENSES;
