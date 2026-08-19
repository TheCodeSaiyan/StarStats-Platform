import React from 'react';
import Link from 'next/link';
import type { Route } from 'next';
import { getSessions, getPlaytime, getUserPlaytime } from '@/lib/api';
import { logger } from '@/lib/logger';
import { rangeToHours } from '@/lib/range';
import { buildSessionSummary, type SessionSummaryLine } from './sessions-summary';
import { lastNSessionDurationsMinutes } from '@/lib/session-series';
import { Sparkline } from '@/components/metrics/Sparkline';
import { defineWidget } from './kit/defineWidget';
import { RankedList, type Row } from './kit/archetypes';
import { fmtDuration } from './kit/format';

/** Mirror of the server's SESSIONS_LIST_LIMIT (event_timeline.rs). The
 *  list endpoint returns at most this many sessions, newest-first. */
const SESSIONS_LIST_CAP = 50;

/** Max session rows shown in the expanded list before "See more". */
const EXPANDED_ROW_CAP = 10;

/**
 * "2h ago", "5d ago" — coarse relative time. For absolute, the row
 * also exposes the ISO timestamp as a `title` tooltip.
 */
function fmtRelative(iso: string): string {
  const then = new Date(iso).getTime();
  if (Number.isNaN(then)) return '';
  const ms = Date.now() - then;
  if (ms < 60_000) return 'just now';
  if (ms < 3_600_000) return `${Math.round(ms / 60_000)}m ago`;
  if (ms < 86_400_000) return `${Math.round(ms / 3_600_000)}h ago`;
  return `${Math.round(ms / 86_400_000)}d ago`;
}

/**
 * SessionSummary from the generated schema: `id`, `started_at?`, `ended_at?`,
 * `event_count`. All four fields are available and rendered.
 */
interface SessionRow {
  id: string;
  started_at?: string | null;
  ended_at?: string | null;
  event_count: number;
}

/** Build the per-session detail link (owner or permitted visitor). */
function sessionHref(handle: string, id: string): Route {
  return (
    `/u/${encodeURIComponent(handle)}/sessions/${encodeURIComponent(id)}`
  ) as Route;
}

/** Duration in ms from a session's start/end, or null when unknown. */
function sessionDurationMs(s: SessionRow): number | null {
  if (s.started_at && s.ended_at) {
    const a = new Date(s.started_at).getTime();
    const b = new Date(s.ended_at).getTime();
    if (!Number.isNaN(a) && !Number.isNaN(b) && b > a) return b - a;
  }
  return null;
}

/** The "when" label for a row — relative start time, or "(open)". */
function sessionWhenLabel(s: SessionRow): string {
  return s.started_at ? fmtRelative(s.started_at) : '(open)';
}

/**
 * Pure render of a single session row — used in the compact
 * last-played mini-card.
 *
 * Design intent:
 * - `s.id` is used ONLY inside the Link `href`, never as visible text.
 * - 2-column grid: left = [relative time over duration], right = [event-count chip].
 * - All colours/radii/borders via CSS custom properties.
 * - Duration formatting goes through the shared kit `fmtDuration` (SECONDS).
 */
function SessionRowLink({ handle, s }: { handle: string; s: SessionRow }) {
  const durationMs = sessionDurationMs(s);
  return (
    <li>
      <Link
        href={sessionHref(handle, s.id)}
        style={{
          display: 'grid',
          gridTemplateColumns: 'minmax(0, 1fr) auto',
          alignItems: 'center',
          padding: '8px 12px',
          background: 'var(--bg-elev)',
          border: '1px solid var(--border)',
          color: 'var(--fg)',
          textDecoration: 'none',
        }}
      >
        <div style={{ display: 'flex', flexDirection: 'column', minWidth: 0 }}>
          <span title={s.started_at ?? undefined} className="hud-trunc">
            {sessionWhenLabel(s)}
          </span>
          <span className="hud-readout hud-readout--dim">
            {durationMs == null ? 'duration unknown' : fmtDuration(durationMs / 1000)}
          </span>
        </div>
        <span className="hud-readout">{s.event_count.toLocaleString()} ev</span>
      </Link>
    </li>
  );
}

/**
 * Summary line ("N sessions · Xh played") with an inline playtime
 * sparkline showing the last-5-sessions duration trend beside it.
 *
 * The sparkline only draws with 2+ plottable sessions — a single point
 * is not a trend. It carries `role="img"` + a label (inside Sparkline)
 * because it conveys a shape the adjacent number does NOT: the totals
 * say how much, the sparkline says which way it's heading.
 */
function SummaryTrendLine({
  summary,
  trend,
  muted,
}: {
  summary: SessionSummaryLine;
  trend: number[];
  muted?: boolean;
}) {
  return (
    <div
      style={{
        display: 'flex',
        alignItems: 'center',
        gap: 10,
        flexWrap: 'wrap',
      }}
    >
      <p
        style={{
          margin: 0,
          ...(muted ? { color: 'var(--fg-muted)', fontSize: 13 } : null),
        }}
      >
        {summary.countLabel}
        {summary.totalHoursLabel ? <> · {summary.totalHoursLabel}</> : null}
      </p>
      {trend.length >= 2 ? (
        <Sparkline
          series={trend}
          label={`session playtime, last ${trend.length} session${trend.length === 1 ? '' : 's'}`}
        />
      ) : null}
    </div>
  );
}

interface SessionsData {
  summary: SessionSummaryLine;
  trend: number[];
  list: SessionRow[];
}

export const sessionsWidget = defineWidget<SessionsData>({
  id: 'sessions',
  eyebrow: 'Sessions',
  rangeAware: true,
  // Plan 4 visitor widening: sessions are available to visitors too — the
  // getSessions endpoint enforces the share_event_timeline toggle
  // server-side, returning 4xx for disallowed access, which `load` converts
  // to null. This is NOT owner-only, so it keeps a custom always-true gate
  // rather than the `visibility` shorthand. The security boundary lives on
  // the server; the client must not narrow it to owner-only here.
  isAvailable(_ctx) {
    return true;
  },
  async load(ctx) {
    // Range-aware: the whole widget follows the dashboard range. `all`
    // resolves to a one-year window (rangeToHours), matching every other
    // range-aware widget — not a true unbounded lifetime.
    const hours = rangeToHours(ctx.range);
    let sessions = null;
    try {
      if (ctx.token) {
        sessions = await getSessions(ctx.token, ctx.ownerHandle, hours);
      }
    } catch (err) {
      logger.warn(
        { err, call: 'widget.sessions', handle: ctx.ownerHandle },
        'sessions fetch failed',
      );
      return null;
    }
    if (!sessions || sessions.sessions.length === 0) return null;

    const list: SessionRow[] = sessions.sessions;

    // Window totals for the summary line, scoped to the SAME range as the
    // session list so the "N sessions · Xh played" headline never
    // contradicts the rows below it. The list endpoint caps at
    // SESSIONS_LIST_CAP newest-first sessions, so summing it undercounts
    // heavy users and pins the count at the cap; the playtime aggregate is
    // exact for the window — me-scoped for the owner, handle-scoped for a
    // visitor (gated by the same share_event_timeline grant as the session
    // list). A visitor WITHOUT the grant 4xxs here and falls back to the
    // capped list, labelled "N+" so the number doesn't read as exact.
    let lifetime: { session_count: number; total_playtime_secs: number } | null = null;
    if (ctx.token) {
      try {
        lifetime = ctx.isOwner
          ? await getPlaytime(ctx.token, hours, false)
          : await getUserPlaytime(ctx.token, ctx.ownerHandle, hours);
      } catch (err) {
        logger.warn(
          {
            err,
            call: 'widget.sessions.playtime',
            handle: ctx.ownerHandle,
            owner: ctx.isOwner,
          },
          'playtime fetch failed',
        );
      }
    }

    // Fallback: sum the durations of the returned (capped) sessions.
    const derivedTotalMs = list.reduce((acc, s) => {
      const ms = sessionDurationMs(s);
      return ms == null ? acc : acc + ms;
    }, 0);
    const summary = buildSessionSummary({
      lifetime,
      listLength: list.length,
      derivedTotalMs,
      listCap: SESSIONS_LIST_CAP,
    });

    // Last-5-sessions playtime trend for the inline sparkline. Derived
    // from the same (newest-first) capped list — no extra fetch. Empty
    // / single-session results simply skip the sparkline.
    const trend = lastNSessionDurationsMinutes(list, 5);

    return { summary, trend, list };
  },
  body(data, ctx, size) {
    const { summary, trend, list } = data;

    // Compact: count + total hours + trend sparkline + one "Last played"
    // mini-card. list[0] is the most recent session (newest-first order).
    if (size === 'compact') {
      const last = list[0];
      return (
        <div style={{ display: 'flex', flexDirection: 'column', gap: 12 }}>
          <SummaryTrendLine summary={summary} trend={trend} />
          <div>
            <div className="hud-tile__eyebrow" style={{ marginBottom: 6 }}>
              Last played
            </div>
            <ul
              style={{
                listStyle: 'none',
                margin: 0,
                padding: 0,
                display: 'flex',
                flexDirection: 'column',
                gap: 8,
              }}
            >
              <SessionRowLink handle={ctx.ownerHandle} s={last} />
            </ul>
          </div>
        </div>
      );
    }

    // Expanded: ranked list of session rows (newest first), capped so the
    // tile never scrolls — the remainder surfaces via "See more".
    const rows: Row[] = list.map((s) => ({
      key: s.id,
      label: (
        <Link href={sessionHref(ctx.ownerHandle, s.id)} title={s.started_at ?? undefined}>
          {sessionWhenLabel(s)}
        </Link>
      ),
      value: `${s.event_count.toLocaleString()} ev`,
    }));
    return (
      <div style={{ display: 'flex', flexDirection: 'column', gap: 12 }}>
        <SummaryTrendLine summary={summary} trend={trend} muted />
        <RankedList
          rows={rows}
          cap={EXPANDED_ROW_CAP}
          seeMore={{
            href: `/u/${encodeURIComponent(ctx.ownerHandle)}/sessions` as Route,
            label: (hidden) => `See all sessions (+${hidden} more) →`,
          }}
        />
      </div>
    );
  },
});
