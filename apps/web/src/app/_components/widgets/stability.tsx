import React from 'react';
import { getPlaytime, getStabilityStats } from '@/lib/api';
import type { StatsBucket } from '@/lib/api';
import { rangeToHours } from '@/lib/range';
import { logger } from '@/lib/logger';
import { defineWidget } from './kit/defineWidget';
import { ReadoutGroup, RankedList, type Readout, type Row } from './kit/archetypes';
import { fmtNum } from './kit/format';

/**
 * `stability` — how often the game falls over on you.
 *
 * `/v1/me/stats/stability` has existed, been tested and had a typed client
 * for as long as the endpoint has; nothing ever called it. It returns the
 * crash count for a window and a split by release channel, both derived from
 * `GameCrash` events the tray already sends.
 *
 * WHY THIS IS NOT ALREADY COVERED. `lives` reports `lives_ended_by_crash` —
 * how many CHARACTER lives ended in a crash. That is a different question
 * from how often the client crashes, and it cannot answer "is this build
 * worse than the last one" because it carries no channel.
 *
 * Owner-only. Crash data is me-scoped with no friend endpoint, so rendering
 * for a visitor would show the VIEWER's crashes on someone else's profile —
 * the same trap `combat_mission` documents.
 */
interface StabilityData {
  crashes: number;
  byChannel: StatsBucket[];
  /**
   * Crashes per hour played, or `null` when we have no playtime to divide by.
   *
   * The rate is the point: a raw count says nothing without knowing whether
   * it covers four hours or forty. Kept as a rate rather than an interval
   * ("one crash every N hours") because the interval is undefined at zero
   * crashes, which is the number a reader most wants to see.
   */
  perHour: number | null;
  hoursPlayed: number;
}

export const stabilityWidget = defineWidget<StabilityData>({
  id: 'stability',
  eyebrow: 'Stability',
  rangeAware: true,
  visibility: 'owner',
  async load(ctx) {
    if (!ctx.isOwner || !ctx.token) return null;
    const token = ctx.token;
    const hours = rangeToHours(ctx.range);
    // Both halves on the SAME window, or the rate is a nonsense: crashes from
    // one period over hours from another.
    const [stabilityRes, playtimeRes] = await Promise.allSettled([
      getStabilityStats(token, hours),
      getPlaytime(token, hours),
    ]);
    if (stabilityRes.status === 'rejected') {
      logger.warn({ err: stabilityRes.reason, call: 'widget.stability' }, 'fetch failed');
      return null;
    }
    if (playtimeRes.status === 'rejected') {
      logger.warn(
        { err: playtimeRes.reason, call: 'widget.stability.playtime' },
        'fetch failed',
      );
    }
    const stability = stabilityRes.value;
    const playtime = playtimeRes.status === 'fulfilled' ? playtimeRes.value : null;
    const hoursPlayed = (playtime?.total_playtime_secs ?? 0) / 3600;

    // A zero-crash window is a RESULT, not an absence — "no crashes" is the
    // best thing this widget can say and hiding it would be perverse. It only
    // returns null when there is no playtime either, i.e. nothing happened at
    // all.
    if (stability.crashes === 0 && hoursPlayed === 0) return null;

    return {
      crashes: stability.crashes,
      byChannel: stability.by_channel ?? [],
      perHour: hoursPlayed > 0 ? stability.crashes / hoursPlayed : null,
      hoursPlayed,
    };
  },
  body(data, _ctx, size) {
    const { crashes, byChannel, perHour, hoursPlayed } = data;
    const readouts: Readout[] = [
      { label: 'crashes', value: fmtNum(crashes) },
      ...(perHour != null
        ? [
            {
              label: 'per hour',
              // Two decimals: a crash rate is usually well under 1, and
              // rounding to whole numbers would render every healthy window
              // as "0".
              value: perHour.toFixed(2),
            } as Readout,
          ]
        : []),
    ];
    const note =
      crashes === 0
        ? `no crashes in ${fmtNum(Math.round(hoursPlayed))}h played`
        : undefined;

    if (size === 'compact' || byChannel.length === 0) {
      return <ReadoutGroup readouts={readouts} note={note} />;
    }
    const rows: Row[] = byChannel.map((b) => ({
      key: b.value,
      label: b.value,
      value: fmtNum(b.count),
    }));
    return (
      <div className="hud-readout-stack">
        <ReadoutGroup readouts={readouts} note={note} />
        <RankedList rows={rows} />
      </div>
    );
  },
});
