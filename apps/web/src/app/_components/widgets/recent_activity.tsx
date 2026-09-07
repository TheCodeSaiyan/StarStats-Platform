import React from 'react';
import { listEvents } from '@/lib/api';
import { formatEventType } from '@/lib/event-types';
import type { ResolvedLocationLike } from '@/lib/event-summary-react';
import { logger } from '@/lib/logger';
import { rangeToSinceIso } from '@/lib/range';
import { defineWidget } from './kit/defineWidget';
import { RankedList } from './kit/archetypes';
import { fmtRelative } from './kit/format';

/**
 * `recent_activity` — the owner's most recent individual events.
 *
 * Owner-only (C2, 2026-07-09): the only source is me-scoped `/v1/me/events`;
 * there is NO friend-scoped event-list equivalent (getFriendTimeline returns
 * aggregated heatmap buckets), so rendering for a visitor would surface the
 * VIEWER's own events on the owner's profile. Gate to owner-only until a
 * `/v1/u/{handle}/events` endpoint exists.
 *
 * Migrated to the kit: `defineWidget` owns fetch/empty/gate; `RankedList`
 * owns the bounded top-N (compact 3, expanded 12 — no see-more link, as
 * there's no natural detail route for a raw event stream).
 *
 * H9: the row label is the event_type run through `formatEventType().label`
 * (curated table → sentence-cased snake fallback) — never the raw snake_case
 * key, which stays addressable in the `title` tooltip.
 */
/**
 * One row of the pane. `payload` and `resolved_location` ride along from
 * `EventDto` so the projection can render the sentence the log actually
 * said (see `me/_projection/recent-activity-rows.tsx`) rather than the type.
 */
export interface RecentActivityEvent {
  seq?: number;
  event_type: string;
  event_timestamp?: string | null;
  payload?: unknown;
  resolved_location?: ResolvedLocationLike | null;
}

export interface RecentActivityData {
  events: ReadonlyArray<RecentActivityEvent>;
}

/**
 * Newest first by EVENT time. `/v1/me/events` orders by ingest sequence,
 * which is not the same thing: the tray uploads in log order within a
 * page, but a session's last few lines can land after events that
 * happened earlier, and the pane claims "most recent first". Events
 * without a timestamp sink to the end; ties keep sequence order.
 */
export function sortNewestFirst<T extends RecentActivityEvent>(
  events: ReadonlyArray<T>,
): T[] {
  const at = (e: T): number => {
    const t = e.event_timestamp ? Date.parse(e.event_timestamp) : Number.NaN;
    return Number.isNaN(t) ? Number.NEGATIVE_INFINITY : t;
  };
  return [...events].sort((a, b) => at(b) - at(a) || (b.seq ?? 0) - (a.seq ?? 0));
}

// Fetch well past the cap: `isLowSignal` drops roughly 70% of a real stream
// (a 320k-event tray database on 2026-09-07 was 30% `attachment_received`,
// 29% `planet_terrain_load`, 13% `hud_notification`), and the projection
// folds runs on top of that. The server clamps at 500.
const FETCH_LIMIT = 200;

/**
 * Types that are instrumentation rather than something the player did.
 * Hidden from this widget only — they still count everywhere else and the
 * session timeline shows them. `join_pu` and friends are already gone by
 * the time `listEvents` returns (see `lib/event-filter.ts`).
 */
const LOW_SIGNAL_TYPES: ReadonlySet<string> = new Set([
  // One row per item on every loadout restore; `burst_summary` covers it.
  'attachment_received',
  // Streaming-in terrain as the player flies past.
  'planet_terrain_load',
  // Opening the inventory screen.
  'location_inventory_requested',
  // The server's reply to `shop_buy_request`, which is the row that matters.
  'shop_flow_response',
]);

/** The one burst rule that summarises activity rather than noise. */
const SIGNAL_BURST_RULES: ReadonlySet<string> = new Set(['loadout_restore_burst']);

function payloadFields(p: unknown): Record<string, unknown> | null {
  return typeof p === 'object' && p !== null ? (p as Record<string, unknown>) : null;
}

/**
 * True for events that would fill the pane without telling the reader
 * anything. HUD banners are kept only when tied to a mission (delivery
 * confirmed, objective text) — armistice-zone and regen reminders are not
 * activity. Bursts are kept only for the loadout rule; the others summarise
 * types this set already hides.
 */
export function isLowSignal(e: RecentActivityEvent): boolean {
  if (LOW_SIGNAL_TYPES.has(e.event_type)) return true;
  const fields = payloadFields(e.payload);
  if (e.event_type === 'hud_notification') {
    const mission = fields?.mission_id;
    return !(typeof mission === 'string' && mission.length > 0);
  }
  if (e.event_type === 'burst_summary') {
    const rule = fields?.rule_id;
    return !(typeof rule === 'string' && SIGNAL_BURST_RULES.has(rule));
  }
  return false;
}
const CAP_COMPACT = 3;
const CAP_EXPANDED = 12;

export const recentActivityWidget = defineWidget<RecentActivityData>({
  id: 'recent_activity',
  eyebrow: 'Recent activity',
  rangeAware: true,
  visibility: 'owner',
  async load(ctx) {
    // Owner-only (see visibility). Defensive: never fetch me-scoped events
    // with a visitor's token even if load is reached directly.
    if (!ctx.isOwner || !ctx.token) return null;
    let events = null;
    try {
      events = await listEvents(ctx.token, {
        limit: FETCH_LIMIT,
        since: rangeToSinceIso(ctx.range),
      });
    } catch (err) {
      logger.warn({ err, call: 'widget.recent_activity' }, 'fetch failed');
      return null;
    }
    const rows = (events?.events ?? []).filter((e) => !isLowSignal(e));
    if (rows.length === 0) return null;
    return { events: sortNewestFirst(rows) };
  },
  body(data, _ctx, size) {
    const now = Date.now();
    const rows = data.events.map((e, i) => ({
      key: String(e.seq ?? i),
      label: (
        <span title={e.event_type}>{formatEventType(e.event_type).label}</span>
      ),
      value: fmtRelative(e.event_timestamp, now),
    }));
    return <RankedList rows={rows} cap={size === 'compact' ? CAP_COMPACT : CAP_EXPANDED} />;
  },
});
