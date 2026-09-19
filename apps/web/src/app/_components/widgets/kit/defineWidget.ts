import type { ReactElement } from 'react';
import type { ViewerCtx, WidgetDef, WidgetId, WidgetSize, WidgetShareScopes } from '../types';
import { LOAD_FAILED, isLoadFailure, type LoadResult } from './loadResult';
import { logger } from '@/lib/logger';

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
  /**
   * Fetch + normalise.
   *
   * `null` means EMPTY — there is genuinely nothing to draw. A FAILURE is a
   * different answer: either throw (this wrapper catches and converts it) or
   * return `LOAD_FAILED` where the failure is caught locally, as it is in the
   * widgets that use `Promise.allSettled` to tolerate a partial outage.
   *
   * Returning `null` for a failed fetch is the bug this contract exists to
   * prevent — see `./loadResult.ts`.
   */
  load: (ctx: ViewerCtx) => Promise<LoadResult<D>>;
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

/**
 * A `WidgetDef` that still remembers what its loader returns.
 *
 * `WidgetDef.load` is deliberately `Promise<unknown | null>` — the registry is
 * a heterogeneous list and the render path does not care. But the projection
 * (`/me`) reuses those loaders and draws the result with its OWN builders, and
 * with `D` erased nothing checked that a builder's parameter type matched the
 * data its widget actually loads. Three builders were caught disagreeing with
 * their widget in production — `docking`, `locations`, `loadout` — each found
 * by a reader or a crash rather than by a gate, because a local interface that
 * contradicts the API compiles perfectly.
 *
 * `__data` is a phantom: optional, never assigned, present only so `D` can be
 * recovered at a call site via {@link WidgetData}. It costs nothing at runtime.
 */
export interface TypedWidgetDef<D> extends WidgetDef {
  readonly __data?: D;
}

/** Recover the loader's data type from a widget defined by `defineWidget`. */
export type WidgetData<W> = W extends TypedWidgetDef<infer D> ? D : never;

export function defineWidget<D>(cfg: WidgetConfig<D>): TypedWidgetDef<D> {
  const isAvailable = cfg.isAvailable ?? gate(cfg.visibility);
  return {
    id: cfg.id,
    eyebrow: cfg.eyebrow,
    defaultSize: cfg.defaultSize ?? 'compact',
    rangeAware: cfg.rangeAware,
    isAvailable,
    async render(ctx, size) {
      const data = await loadSafely(cfg, ctx);
      // The flat surface still draws nothing for either outcome; the
      // projection is what tells them apart. Keeping this branch collapsed
      // means no behaviour change on `/u/[handle]` from this commit.
      if (data == null || isLoadFailure(data)) return null;
      return cfg.body(data, ctx, size);
    },
    // Exposed so a DIFFERENT render layer can reuse the same fetch.
    //
    // The projection (`/me`) draws the same data as callouts, SubStats and
    // ranked Planes rather than as flat widget tiles — but every endpoint call,
    // empty-check, trend computation and provenance caveat in `load` is still
    // exactly right, and duplicating that per element would be 13 near-copies
    // drifting apart. So `load` comes out and the projection supplies its own
    // body; `render` is untouched and the flat profile surface keeps working.
    load: ((ctx: ViewerCtx) => loadSafely(cfg, ctx)) as (
      ctx: ViewerCtx,
    ) => Promise<unknown | null>,
  };
}

/**
 * Run a widget's loader so a THROWN error becomes `LOAD_FAILED` rather than
 * escaping into `Promise.allSettled` (where a rejection is indistinguishable
 * from a widget that chose to render nothing).
 *
 * The `call: widget.<id>` log label matches what the widgets' own catch
 * blocks emitted, so existing log searches keep working.
 */
async function loadSafely<D>(
  cfg: WidgetConfig<D>,
  ctx: ViewerCtx,
): Promise<LoadResult<D>> {
  try {
    return await cfg.load(ctx);
  } catch (err) {
    logger.warn({ err, call: `widget.${cfg.id}` }, 'widget load failed');
    return LOAD_FAILED;
  }
}
