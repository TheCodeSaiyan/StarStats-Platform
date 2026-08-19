import React from 'react';
import { Provenance } from '@/components/Provenance';
import { InstrumentStrip } from '@/components/hud/InstrumentStrip';

/**
 * `/me` identity header — the "who am I" block at the top of the
 * private home page.
 *
 * PURE presentational component: it takes already-fetched, already-
 * derived data as props and does no fetching of its own. The `/me`
 * page owns the `Promise.allSettled` fetch + derivation and feeds the
 * finished numbers here. Keeping it pure makes it unit-testable with
 * plain `@testing-library/react` (no network, no server-component
 * async) and keeps it range-INDEPENDENT: `me/page.tsx` fetches the
 * windowed metrics (playtime/locations/deaths/kills) with the server's
 * MAX window, and `totalEvents` is an all-time count — so the header is
 * a stable "who am I" anchor and the RangeBar below it never re-renders
 * this block. ("Lifetime" is best-available: the windowed endpoints cap
 * at ~1 year / STATS_MAX_HOURS.)
 *
 * Rendered via `<InstrumentStrip>`: title = `@{handle}`, context holds
 * the supporter chip + enlistment year, readouts carry the four lifetime
 * stats (play / events / loc / k/d).
 */

interface Props {
  /** The signed-in user's claimed handle (case-preserved from RSI). */
  handle: string;
  /**
   * Supporter tier key (`coffee` / `standard` / `generous` or a
   * future label), or null when the user isn't a supporter. Rendered
   * as a small chip next to the handle.
   */
  supporterTier: string | null;
  /**
   * RSI "Enlisted" date (`YYYY-MM-DD`) from the citizen profile
   * snapshot, or null when no snapshot exists yet. Only the year is
   * surfaced ("Citizen since 2021").
   */
  enlistmentDate: string | null;
  /** All-time total captured events (from the summary endpoint). */
  totalEvents: number;
  /** Lifetime deaths (combat stats endpoint, max window). */
  deaths: number;
  /** How many of `deaths` were inferred rather than observed. K/D is
   *  derived from deaths, so a partly-reconstructed death count makes
   *  the RATIO partly a guess — which the reader should be able to see. */
  deathsInferred?: number;
  /** Lifetime kills (combat stats: actor_death where killer == handle, max window). */
  kills: number;
  /** Lifetime playtime in seconds (max window). Formatted as whole hours. */
  playtimeSecs: number;
  /** Distinct locations visited (lifetime, max window). */
  locationsVisited: number;
}

/** Whole-hours formatter: 418*3600s -> "418h", 90min -> "2h". */
export function formatPlaytime(secs: number): string {
  return `${Math.round(secs / 3600)}h`;
}

/** K/D ratio. When deaths is 0 we can't divide, so we surface the raw
 *  kill count instead of Infinity. Otherwise one decimal place. */
export function formatKd(kills: number, deaths: number): string {
  return deaths ? (kills / deaths).toFixed(1) : String(kills);
}

/** Pull the year off an ISO `YYYY-MM-DD` (or full ISO) date string. */
export function enlistmentYear(date: string | null): string | null {
  if (!date) return null;
  const year = date.slice(0, 4);
  return /^\d{4}$/.test(year) ? year : null;
}

export function MeIdentityHeader({
  handle,
  supporterTier,
  enlistmentDate,
  totalEvents,
  deaths,
  deathsInferred = 0,
  kills,
  playtimeSecs,
  locationsVisited,
}: Props) {
  const year = enlistmentYear(enlistmentDate);

  const context =
    supporterTier || year ? (
      <span style={{ display: 'inline-flex', gap: 8, alignItems: 'center' }}>
        {supporterTier && (
          <span className="ss-badge ss-badge--accent">
            <span className="ss-badge-dot" />
            {supporterTier} supporter
          </span>
        )}
        {year && (
          <span style={{ color: 'var(--fg-muted)', fontSize: 12 }}>
            Citizen since {year}
          </span>
        )}
      </span>
    ) : undefined;

  return (
    <InstrumentStrip
      title={
        // Real <h1> so `/me` has a page heading (WCAG / landmark
        // navigation). `inherit` keeps it at the InstrumentStrip's
        // 14px so the visual is unchanged from the previous plain
        // string title.
        <h1
          className="hud-tile__title"
          style={{ margin: 0, fontSize: 'inherit', fontWeight: 'inherit' }}
        >
          @{handle}
        </h1>
      }
      context={context}
      readouts={[
        { k: 'play', v: formatPlaytime(playtimeSecs) },
        { k: 'events', v: totalEvents.toLocaleString() },
        { k: 'loc', v: locationsVisited.toLocaleString() },
        {
          k: 'k/d',
          v: (
            <Provenance
              total={deaths}
              inferred={deathsInferred}
              note="derived from deaths, some reconstructed from Corpse lines as the game no longer logs them directly"
            >
              {formatKd(kills, deaths)}
            </Provenance>
          ),
        },
      ]}
    />
  );
}
