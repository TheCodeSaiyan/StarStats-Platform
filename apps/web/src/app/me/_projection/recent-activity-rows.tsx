import React from 'react';
import { formatEventType, groupLabel, type EventGroup } from '@/lib/event-types';
import { formatEventSummary } from '@/lib/event-summary';
import { renderEventSummary } from '@/lib/event-summary-react';
import type { ReferenceCatalogs, ReferenceLookup } from '@/lib/reference-types';
import {
  sortNewestFirst,
  type RecentActivityEvent,
} from '@/app/_components/widgets/recent_activity';

/**
 * Rows for the projection's "Recent activity" pane.
 *
 * The pane used to print the raw `event_type` and a bare clock time, so a
 * reader saw `attachment_received · 03:21 PM` eight times over and learned
 * nothing — the identifier, not the event, and yesterday's events dressed as
 * today's. Each row now carries what the log actually said:
 *
 *   - the SENTENCE the payload renders to (`renderEventSummary`, the same
 *     per-variant formatter the tray's timeline uses, with KB links where the
 *     catalogue knows the entity), falling back to the curated verb label and
 *     never to the snake_case key — which stays reachable as the tooltip;
 *   - a day-aware time: the clock for today, the date for anything older;
 *   - the event's group in the mark column, and a warning tone on combat.
 *
 * Pure: no fetching, no hooks. Kept out of `elements.tsx` so it can be tested
 * without dragging every widget loader into the test.
 */

export interface RecentRefs {
  lookup?: ReferenceLookup;
  catalogs?: ReferenceCatalogs;
}

export interface RecentRow {
  key: string;
  time: React.ReactNode;
  event: React.ReactNode;
  tone?: 'bad';
  mark: React.ReactNode;
  /** The event behind the row — the newest member when a run was folded. */
  source: RecentActivityEvent;
  /** How many events the row stands for (1 unless folded). */
  count: number;
}

const MISSING = '—';

/**
 * Fixed three-letter months rather than the locale's: `en-GB` spells
 * September "Sept", and "30 Sept 23:59" measured 67px in the log row's
 * 64px time cell on the live page, where "30 Sep 23:59" is 63px. The time
 * zone is the server's, as it was before this existed — the full ISO
 * timestamp goes in the tooltip so the reader can check.
 */
const MONTHS = [
  'Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun',
  'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec',
] as const;

function clock(d: Date): string {
  const hh = String(d.getHours()).padStart(2, '0');
  const mm = String(d.getMinutes()).padStart(2, '0');
  return `${hh}:${mm}`;
}

export function fmtLogTime(
  iso: string | null | undefined,
  now: Date,
  opts: {
    /** Keep the clock on older days too — the log page wants it, the
     *  eight-row digest does not. */
    clock?: boolean;
  } = {},
): { text: string; title: string } {
  if (!iso) return { text: MISSING, title: '' };
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return { text: MISSING, title: '' };
  const sameDay = d.toDateString() === now.toDateString();
  if (sameDay) return { text: clock(d), title: iso };
  const day = `${d.getDate()} ${MONTHS[d.getMonth()]}`;
  return { text: opts.clock ? `${day} ${clock(d)}` : day, title: iso };
}

function hasTypedPayload(p: unknown): boolean {
  return (
    typeof p === 'object' &&
    p !== null &&
    'type' in p &&
    typeof (p as { type: unknown }).type === 'string'
  );
}

function isRenderable(node: React.ReactNode): boolean {
  return node != null && node !== '' && node !== false;
}

/**
 * Groups whose consecutive repeats fold into one row with a count. Combat,
 * missions and session boundaries never fold: two deaths are two rows even
 * when the sentences match, because each one is the point.
 */
const FOLDABLE_GROUPS: ReadonlySet<EventGroup> = new Set([
  'vehicle',
  'loadout',
  'commerce',
  'travel',
  'system',
]);

/**
 * Two adjacent events fold when they would read the same. The key is the
 * STRING rendering of the payload, so "Now at Lorville" twice folds and
 * "Now at Lorville" then "Now at Everus Harbor" does not. `vehicle_stowed`
 * folds on type alone: its sentence differs only by an engine vehicle id,
 * and a hangar stowing six ships at once is one thing that happened.
 */
function foldKey(e: RecentActivityEvent, refs: RecentRefs | undefined): string | null {
  const meta = formatEventType(e.event_type);
  if (!FOLDABLE_GROUPS.has(meta.group)) return null;
  if (e.event_type === 'vehicle_stowed') return e.event_type;
  const sentence = hasTypedPayload(e.payload)
    ? formatEventSummary(e.payload, refs?.lookup)
    : '';
  return `${e.event_type}|${sentence}`;
}

interface Run {
  /** Newest member — rows are built from the list sorted newest first. */
  anchor: RecentActivityEvent;
  members: RecentActivityEvent[];
  /** Whether every member renders to the same sentence. */
  uniform: boolean;
}

function foldRuns(
  ordered: ReadonlyArray<RecentActivityEvent>,
  refs: RecentRefs | undefined,
): Run[] {
  const runs: Run[] = [];
  let lastKey: string | null = null;
  for (const e of ordered) {
    const key = foldKey(e, refs);
    const last = runs.length > 0 ? runs[runs.length - 1] : null;
    if (key !== null && last && key === lastKey) {
      last.members.push(e);
      // A type-only key (vehicle_stowed) can still be uniform in prose.
      if (last.uniform && e.event_type === 'vehicle_stowed') {
        const a = hasTypedPayload(last.anchor.payload)
          ? formatEventSummary(last.anchor.payload, refs?.lookup)
          : '';
        const b = hasTypedPayload(e.payload) ? formatEventSummary(e.payload, refs?.lookup) : '';
        last.uniform = a === b;
      }
    } else {
      runs.push({ anchor: e, members: [e], uniform: true });
    }
    lastKey = key;
  }
  return runs;
}

export function recentActivityRows(
  events: ReadonlyArray<RecentActivityEvent>,
  refs: RecentRefs | undefined,
  opts: {
    now: Date;
    cap?: number;
    /** Fold consecutive repeats (default). `/me/activity` passes `false`:
     *  a log page owes the reader every line, not a digest of them. */
    fold?: boolean;
    /** Show the clock on older days too — see `fmtLogTime`. */
    clock?: boolean;
  },
): RecentRow[] {
  const ordered = sortNewestFirst(events);
  const runs =
    opts.fold === false
      ? ordered.map((e) => ({ anchor: e, members: [e], uniform: true }))
      : foldRuns(ordered, refs);
  const shown = typeof opts.cap === 'number' ? runs.slice(0, opts.cap) : runs;
  return shown.map((run, i) => {
    const e = run.anchor;
    const n = run.members.length;
    const meta = formatEventType(e.event_type);
    const sentence =
      hasTypedPayload(e.payload) && (n === 1 || run.uniform)
        ? renderEventSummary(
            e.payload,
            refs?.lookup,
            refs?.catalogs,
            e.resolved_location ?? null,
          )
        : null;
    const time = fmtLogTime(e.event_timestamp, opts.now, { clock: opts.clock });
    const oldest = run.members[n - 1];
    const timeTitle =
      n > 1 && oldest.event_timestamp
        ? `${n} events, ${oldest.event_timestamp} to ${time.title}`
        : time.title;
    return {
      key: String(e.seq ?? i),
      time: <span title={timeTitle}>{time.text}</span>,
      event: (
        <span title={n > 1 ? `${e.event_type} ×${n}` : e.event_type}>
          {isRenderable(sentence) ? sentence : meta.label}
          {n > 1 ? ` ×${n}` : null}
        </span>
      ),
      tone: meta.group === 'combat' ? 'bad' : undefined,
      mark: groupLabel(meta.group),
      source: e,
      count: n,
    };
  });
}
