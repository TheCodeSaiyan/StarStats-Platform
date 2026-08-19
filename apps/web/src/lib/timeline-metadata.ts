/**
 * Timeline grouping + folding helpers for `EventEnvelope.metadata`.
 *
 * Web mirror of `apps/tray-ui/src/timeline/grouping.ts`. The two
 * surfaces share an event shape (the generated `EventEnvelope`) but
 * Next.js can't import from the tray package (different bundler,
 * different `tsconfig`), so the helpers are intentionally duplicated.
 *
 * Why duplicate rather than extract to a shared package: the helpers
 * total ~70 lines, both surfaces are still iterating on the v2 event
 * pipeline (Phase 5), and splitting them now would block the web app
 * on tray-side refactors. When/if the helpers stabilise, fold them
 * into a `packages/event-metadata` workspace package.
 *
 * Both helpers are pure — they take wire `EventEnvelope[]` and return
 * view-ready shapes. Component code stays presentational; testability
 * stays high.
 *
 * Note: `apps/web` does not currently expose a per-event timeline
 * endpoint (only the day-bucketed `PublicTimelineResponse`). These
 * helpers are landed ahead of the consumer so the moment a per-event
 * shared-timeline endpoint ships, the shared timeline can adopt
 * `metadata.primary_entity` / `metadata.group_key` without further
 * refactor work. See `the release design notesfollow-ups/` for tracking.
 */

import type { EventEnvelope, EntityRef } from 'api-client-ts';
import { formatEventType } from './event-types';

export interface TimelineRow {
  /** Group key the row was folded on. Falls back to the envelope's
   *  `idempotency_key` prefixed with `__` when the envelope has no
   *  metadata, so rows still stay unique on screen. */
  key: string;
  /** Number of envelopes folded into this row. `1` means an
   *  unfolded row; >1 means the count badge should render. */
  count: number;
  /** All envelopes folded into the row, in input order. */
  members: EventEnvelope[];
  /** First envelope in the run — the one the row's summary is drawn
   *  from. The rest are revealed on drill-in. */
  anchor: EventEnvelope;
}

export interface EntitySection {
  /** Primary entity all events in this section are about. */
  entity: EntityRef;
  /** Timestamp of the most recent event in the section. Used to sort
   *  sections — newest first. Empty string when no event in the
   *  section carries an `event.timestamp`. */
  lastActivity: string;
  /** Adjacent-folded rows derived from `events`. */
  rows: TimelineRow[];
  /** Raw envelopes assigned to this section, in input order. */
  events: EventEnvelope[];
}

/**
 * Collapse runs of envelopes with the same `metadata.group_key` into
 * a single row carrying a `count` and the full member list. Only
 * adjacent envelopes fold — an interruption by a different key
 * breaks the run, mirroring how `Game.log` is read top-to-bottom.
 *
 * Envelopes without metadata get a unique key derived from their
 * idempotency key, which guarantees they never fold with neighbours.
 */
export function foldAdjacentSameKey(events: EventEnvelope[]): TimelineRow[] {
  const rows: TimelineRow[] = [];
  for (const ev of events) {
    const key = ev.metadata?.group_key ?? `__${ev.idempotency_key}`;
    const last = rows.length > 0 ? rows[rows.length - 1] : null;
    if (last && last.key === key) {
      rows[rows.length - 1] = {
        ...last,
        count: last.count + 1,
        members: [...last.members, ev],
      };
    } else {
      rows.push({ key, count: 1, members: [ev], anchor: ev });
    }
  }
  return rows;
}

function entityKey(entity: EntityRef): string {
  return `${entity.kind}:${entity.id}`;
}

/**
 * Group envelopes by their primary entity (`kind:id`). Within each
 * section the events keep input order; sections themselves are
 * sorted by `lastActivity` (the newest `event.timestamp` in the
 * bucket), newest first.
 *
 * Envelopes without `metadata.primary_entity` are silently dropped:
 * the entity-first view has nothing to anchor them on. The
 * chronological view (see `foldAdjacentSameKey`) is the fallback for
 * those.
 */
export function groupEventsForTimeline(
  events: EventEnvelope[]
): EntitySection[] {
  const byEntity = new Map<string, EntitySection>();
  for (const ev of events) {
    const entity = ev.metadata?.primary_entity;
    if (entity == null) continue;
    const id = entityKey(entity);
    // Every `GameEvent` variant carries a `timestamp: string` field,
    // so the tagged-union narrowing on the generated type lets us
    // read it directly.
    const ts = ev.event?.timestamp ?? '';
    const existing = byEntity.get(id);
    if (existing == null) {
      byEntity.set(id, {
        entity,
        lastActivity: ts,
        rows: [],
        events: [ev],
      });
    } else {
      const lastActivity =
        ts > existing.lastActivity ? ts : existing.lastActivity;
      byEntity.set(id, {
        ...existing,
        lastActivity,
        events: [...existing.events, ev],
      });
    }
  }

  const sections = Array.from(byEntity.values()).map((s) => ({
    ...s,
    rows: foldAdjacentSameKey(s.events),
  }));
  sections.sort((a, b) => b.lastActivity.localeCompare(a.lastActivity));
  return sections;
}

/**
 * Closed vocabulary of entity kinds, mirroring
 * `starstats-core::metadata::EntityKind`. Centralised here so any
 * list / filter / detail view consumes the same list rather than
 * hand-rolling string arrays at the call site.
 */
export const ENTITY_KINDS = [
  'player',
  'vehicle',
  'item',
  'location',
  'shop',
  'mission',
  'session',
  'system',
] as const;

export type EntityKindLiteral = (typeof ENTITY_KINDS)[number];

/**
 * Human-readable label for an entity kind. The wire format is
 * snake_case (`vehicle`, `shop`); the UI label is Title Case. Pure
 * lookup — falls back to the raw kind string when an unknown kind
 * arrives (e.g. older client + newer server).
 */
export function labelForEntityKind(kind: string): string {
  const map: Record<string, string> = {
    player: 'Player',
    vehicle: 'Vehicle',
    item: 'Item',
    location: 'Location',
    shop: 'Shop',
    mission: 'Mission',
    session: 'Session',
    system: 'System',
  };
  return map[kind] ?? kind;
}

/**
 * Display name for an envelope's primary entity, falling back to the
 * `event-types` registry's human label when no metadata is attached.
 * Centralised so the shared timeline (when it lands) and any list
 * view consume the same rule — and so the raw snake_case event_type
 * never bleeds through as a row title.
 *
 * Resolution order:
 *   1. `metadata.primary_entity.display_name` (server-stamped)
 *   2. `formatEventType(event.type).label` (registry verb phrase)
 *   3. Bare `"Event"` when even the type discriminant is missing.
 */
export function rowTitleForEnvelope(envelope: EventEnvelope): string {
  const entity = envelope.metadata?.primary_entity;
  if (entity != null && entity.display_name.trim()) {
    return entity.display_name;
  }
  const type = envelope.event?.type;
  if (type) {
    return formatEventType(type).label;
  }
  return 'Event';
}
