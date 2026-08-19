import React from 'react';
import { DayHeatmap } from '@/components/DayHeatmap';
import { getTimeline, getFriendTimeline } from '@/lib/api';
import { logger } from '@/lib/logger';
import { rangeToDays } from '@/lib/range';
import { defineWidget } from './kit/defineWidget';

/**
 * `heatmap` — "Activity shape": a per-day event-count grid over the
 * active range. Migrated to the kit: `defineWidget` owns the
 * fetch/empty/gate boilerplate; the body stays the bespoke
 * `<DayHeatmap>` render (NOT a kit archetype).
 *
 * Not owner-only — it has a visitor path (owner `getTimeline` vs
 * visitor `getFriendTimeline`). The existing card had no explicit
 * share-toggle gate, so `isAvailable` stays `() => true`: visibility is
 * server-gated (a visitor without permission simply gets no timeline
 * data, and `load` returns null → the tile auto-collapses).
 */
type HeatmapData =
  | Awaited<ReturnType<typeof getTimeline>>
  | Awaited<ReturnType<typeof getFriendTimeline>>;

export const heatmapWidget = defineWidget<HeatmapData>({
  id: 'heatmap',
  eyebrow: 'Activity shape',
  defaultSize: 'expanded',
  rangeAware: true,
  // No explicit share-toggle gate — visible to anyone the visitor
  // share-scope permits, same as today. Match that by always returning
  // true; visitors without permission won't get timeline data anyway.
  isAvailable: () => true,
  async load(ctx) {
    const days = rangeToDays(ctx.range);
    let timeline: HeatmapData | null = null;
    try {
      if (ctx.token && ctx.isOwner) {
        timeline = await getTimeline(ctx.token, { days });
      } else if (ctx.token) {
        timeline = await getFriendTimeline(ctx.token, ctx.ownerHandle, days);
      }
    } catch (err) {
      logger.warn(
        { err, call: 'widget.heatmap', handle: ctx.ownerHandle },
        'heatmap fetch failed',
      );
      return null;
    }
    if (!timeline || timeline.buckets.length === 0) return null;
    return timeline;
  },
  body(data) {
    return <DayHeatmap timeline={data} />;
  },
});
