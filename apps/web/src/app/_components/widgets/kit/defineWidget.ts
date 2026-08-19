import type { ReactElement } from 'react';
import type { ViewerCtx, WidgetDef, WidgetId, WidgetSize, WidgetShareScopes } from '../types';

/**
 * Declarative widget definition. Collapses the boilerplate every widget
 * repeated by hand — the fetch → "null on error/empty → bail" dance and the
 * `WidgetDef` shape — into config, so a widget file is just: what it needs,
 * who can see it, and how to draw the (already-bounded) summary.
 *
 *   defineWidget({
 *     id: 'fleet', eyebrow: 'Fleet', visibility: 'owner',
 *     load: (ctx) => getFleet(ctx.token).then(r => r.ships.length ? r : null),
 *     body: (data, ctx) => <RankedList rows={…} cap={6} seeMore={…} />,
 *   })
 *
 * `load` owns fetching + the empty check (return null to render nothing).
 * `body` is pure presentation, composing the shared archetype renderers.
 * Security gates stay explicit via `visibility` (or a custom `isAvailable`)
 * — never inferred — because leaking a visitor the owner's me-scoped data is
 * the exact bug the C2 audit fixed.
 */
export type Visibility =
  | 'owner' // owner-only (me-scoped data with no friend endpoint)
  | 'public' // anyone (public data)
  | { shareScope: keyof WidgetShareScopes }; // owner OR a granted share toggle

export interface WidgetConfig<D> {
  id: WidgetId;
  eyebrow: string;
  defaultSize?: WidgetSize;
  rangeAware?: boolean;
  /** Who may see this widget. Use a custom `isAvailable` for anything more
   *  involved than these three shapes. */
  visibility?: Visibility;
  /** Escape hatch for bespoke gates; wins over `visibility` when set. */
  isAvailable?: (ctx: ViewerCtx) => boolean | Promise<boolean>;
  /** Fetch + normalise. Return `null` for "no data / error" → renders
   *  nothing (WidgetFrame shows the shared empty placeholder). */
  load: (ctx: ViewerCtx) => Promise<D | null>;
  /** Draw the bounded summary from the loaded data. Pure — compose the
   *  archetype renderers. May itself return null (defensive). */
  body: (data: D, ctx: ViewerCtx, size: WidgetSize) => ReactElement | null;
}

function gate(visibility: Visibility | undefined) {
  return (ctx: ViewerCtx): boolean => {
    if (visibility === 'public') return true;
    if (visibility === undefined || visibility === 'owner') return ctx.isOwner;
    // { shareScope }: the owner always sees it; a visitor only if the owner
    // granted that share toggle for them.
    return ctx.isOwner || ctx.shareScopes[visibility.shareScope] === true;
  };
}

export function defineWidget<D>(cfg: WidgetConfig<D>): WidgetDef {
  const isAvailable = cfg.isAvailable ?? gate(cfg.visibility);
  return {
    id: cfg.id,
    eyebrow: cfg.eyebrow,
    defaultSize: cfg.defaultSize ?? 'compact',
    rangeAware: cfg.rangeAware,
    isAvailable,
    async render(ctx, size) {
      const data = await cfg.load(ctx);
      if (data == null) return null;
      return cfg.body(data, ctx, size);
    },
  };
}
