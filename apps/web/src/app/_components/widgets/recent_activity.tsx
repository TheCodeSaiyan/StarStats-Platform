import React from 'react';
import { listEvents } from '@/lib/api';
import { formatEventType } from '@/lib/event-types';
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
interface RecentActivityData {
  events: ReadonlyArray<{
    seq?: number;
    event_type: string;
    event_timestamp?: string | null;
  }>;
}

// Fetch enough to fill the expanded cap; body caps per size.
const FETCH_LIMIT = 20;
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
    const rows = events?.events ?? [];
    if (rows.length === 0) return null;
    return { events: rows };
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
