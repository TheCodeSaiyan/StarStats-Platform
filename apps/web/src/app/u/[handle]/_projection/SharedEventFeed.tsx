/**
 * The events behind a share, on the owner's profile.
 *
 * Until `GET /v1/u/{handle}/events` existed, every friend-scoped read
 * returned aggregates, so this page could say a pilot had logged 4,000
 * events and 12 distinct types and still not show a single one of them.
 *
 * Rows come from `recentActivityRows` — the same builder `/me/activity`
 * uses — rather than a renderer of its own. That is deliberate: the
 * headline rules (`humanTitleForEntry`'s layered fallback, `EntityLink`
 * on entity identifiers, the tone chip carrying the raw type) are the
 * ones the tray and the owner's own log already follow, and a second
 * implementation is a second place for them to drift.
 *
 * The server has already applied the share's clamps by the time these
 * events arrive: hidden rows are gone, the scope's allow/deny types are
 * enforced, and the window is clamped to the share. Nothing here is a
 * privacy control — this is presentation only.
 */
import React from 'react';
import { Plane, HoloKV, Flatline } from 'holo';
import {
  recentActivityRows,
  type RecentRefs,
} from '@/app/me/_projection/recent-activity-rows';
import { eventDetailItems } from '@/app/me/activity/_lib/details';
import { isLowSignal } from '@/app/_components/widgets/recent_activity';
import type { SharedEventDto } from '@/lib/api';

export interface SharedEventFeedProps {
  events: SharedEventDto[];
  /** Catalogue lookups for `EntityLink`. Absent degrades to plain text. */
  refs: RecentRefs | undefined;
}

export function SharedEventFeed({ events, refs }: SharedEventFeedProps) {
  // Same default as `/me/activity` and the recent-activity widget:
  // In-Transit movement chatter is suppressed at the render layer. The
  // owner's own log offers a switch to show it; a recipient reading
  // someone else's profile has no such control, so the quieter default
  // is the only one on offer here.
  const visible = events.filter((e) => !isLowSignal(e));

  const rows = recentActivityRows(visible, refs, {
    now: new Date(),
    // A log owes the reader every line it shows, not a digest — the
    // same call `/me/activity` makes.
    fold: false,
    clock: true,
  });

  if (rows.length === 0) {
    return <Flatline reason="no-data" />;
  }

  return (
    <Plane
      tilt="flat"
      cap="Shared events"
      hint="newest first · open a row for the record"
    >
      {rows.map((r) => (
        <details key={r.key} className="hp-lg-x">
          <summary className="hp-lg">
            <span className="t">{r.time}</span>
            <span className={['ev', r.tone].filter(Boolean).join(' ')}>
              {r.event}
            </span>
            <span className="mx">{r.mark}</span>
          </summary>
          <div className="hp-lg-x__body">
            <HoloKV items={eventDetailItems(r.source)} />
          </div>
        </details>
      ))}
    </Plane>
  );
}
