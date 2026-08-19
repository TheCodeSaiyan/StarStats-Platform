import 'server-only';

import type { LayoutSurface } from '@/lib/api';
import { logger } from '@/lib/logger';
import { getProfileLayoutForRender } from '@/lib/profile-layout';
import { NoSignal } from '@/components/hud/NoSignal';
import { WIDGETS_BY_ID } from './registry';
import { effectiveSize } from './grid-layout';
import { titleForWidget } from './widget-meta';
import type { ViewerCtx, WidgetDef } from './types';
import {
  SortableProfileWidgets,
  type RenderedWidget,
} from './SortableProfileWidgets';

/**
 * Shared widget grid used by BOTH the public profile (`/u/[handle]`,
 * `surface="profile"`) and the private home page (`/me`,
 * `surface="home"`).
 *
 * Extracted from `u/[handle]/page.tsx`'s former `ProfileWidgetGrid`
 * so the two surfaces share one render path. The ONLY surface-specific
 * behaviour is:
 *   - which default layout is used when the user has no stored layout
 *     (`getProfileLayoutForRender(..., surface)` picks
 *     HOME_DEFAULT_LAYOUT vs DEFAULT_LAYOUT), and
 *   - which layout column the owner's drag-reorder saves into
 *     (`SortableProfileWidgets surface` -> `saveProfileLayoutAction`).
 *
 * Everything else (availability gating, per-widget render with
 * `Promise.allSettled`, the owner edit toolbar) is identical, so the
 * profile page keeps behaving exactly as before when it passes
 * `surface="profile"` (the default).
 */
export async function WidgetCanvas({
  ctx,
  surface = 'profile',
}: {
  ctx: ViewerCtx;
  surface?: LayoutSurface;
}) {
  const layout = await getProfileLayoutForRender(
    ctx.token,
    ctx.ownerHandle,
    ctx.isOwner,
    surface,
  );
  const renderedMap = new Map<string, RenderedWidget>();
  const settled = await Promise.allSettled(
    layout.map(async (entry) => {
      const def = WIDGETS_BY_ID.get(entry.id as WidgetDef['id']);
      if (!def) return null;
      const available = await def.isAvailable(ctx);
      if (!available) return null;
      // Render at the size the tile's real WIDTH can hold, not just the
      // stored flag — a tile dragged wider should show more, and until now
      // its dimensions never reached the body. Monotonic, so an explicitly
      // expanded tile stays expanded at any width. `entry.w` is absent on a
      // never-arranged layout, in which case this is exactly `entry.size`.
      const body = await def.render(ctx, effectiveSize(entry.size, entry.w ?? 0));
      return {
        id: entry.id,
        eyebrow: def.eyebrow,
        title: titleForWidget(def.id),
        body: body ?? <NoSignal compact />,
        // A widget with no data renders a compact "no signal" placeholder;
        // the grid collapses such tiles to a short height in view mode so
        // an empty widget never leaves a full-size empty box.
        empty: body == null,
        isRangeAware: def.rangeAware ?? false,
      } satisfies RenderedWidget;
    }),
  );
  settled.forEach((r, i) => {
    if (r.status === 'rejected') {
      logger.warn(
        { err: r.reason, idx: i, call: 'widget.render' },
        'widget render failed',
      );
      return;
    }
    if (r.value) renderedMap.set(r.value.id, r.value);
  });
  return (
    <>
      <SortableProfileWidgets
        initialLayout={layout}
        rendered={renderedMap}
        surface={surface}
        lensEnabled={surface === 'home'}
      />
    </>
  );
}
