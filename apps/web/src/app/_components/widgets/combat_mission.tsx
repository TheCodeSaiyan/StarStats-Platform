import React from 'react';
import { getCombatStats, getMetricsEventTypes, getObjectives } from '@/lib/api';
import type { ObjectivesResponse, StatsBucket } from '@/lib/api';
import { rangeToMetricsRange, rangeToHours } from '@/lib/range';
import { logger } from '@/lib/logger';
import { defineWidget } from './kit/defineWidget';
import { ReadoutGroup, RankedList, type Readout, type Row } from './kit/archetypes';
import { fmtNum, countsByType, sumCounts } from './kit/format';

/**
 * `combat_mission` — deaths, vehicle losses, mission throughput, and
 * objective outcomes for the active range.
 *
 * Owner-only (C2, 2026-07-09): the only data source is the me-scoped
 * `/v1/me/metrics/event-types` — there is NO friend-scoped equivalent,
 * so rendering for a visitor would surface the VIEWER's own combat
 * metrics on the owner's profile. `visibility: 'owner'` gates it, and
 * `load` re-guards defensively. Do NOT reinstate a
 * `shareScopes.combat_mission` visitor path without a real friend endpoint.
 */
/**
 * FALLBACK ONLY. Counting deaths by summing event types cannot be right:
 * `actor_death` fires whether the caller was the victim OR the killer, so
 * every kill a reader scored was being added to their death total. Measured
 * against a fixture the server scored as 21 kills / 12 deaths, this widget
 * displayed "Deaths 21".
 *
 * `player_incapacitated` was in here too — downed but alive, counted as dead.
 *
 * `/v1/me/stats/combat` does the separation properly (its own comment: "kills
 * = actor_death where the caller is the killer, deaths = actor_death where
 * the caller is the victim", unioned with `player_death` for modern builds),
 * so that is the source now. This list survives only for when that call
 * fails, where an over-count beats a blank.
 */
const DEATH_TYPES_FALLBACK = ['player_death', 'actor_death'];
const INCAPACITATED_TYPES = ['player_incapacitated'];
const VEHICLE_LOSS_TYPES = ['vehicle_destruction'];
const MISSION_START_TYPES = ['mission_start'];
const MISSION_END_TYPES = ['mission_end'];

interface CombatMissionData {
  deaths: number;
  /** Server-computed kills. `null` when the combat call failed. */
  kills: number | null;
  /** Downed but not killed — never folded into `deaths`, which is what this
   *  widget used to do. */
  incapacitated: number;
  vehicleLosses: number;
  missionsStarted: number;
  missionsEnded: number;
  completionPct: number | null;
  objectivePct: number | null;
  counts: Record<string, number>;
  objectives: ObjectivesResponse | null;
  /**
   * Weapon → kill count, and zone → death count.
   *
   * `CombatStatsResponse` has carried both since it was written and nothing
   * ever rendered them: `/me` fetched the response, destructured `kills` and
   * `deaths`, and dropped these on the floor. No new query, no new capture —
   * only a caller.
   *
   * `top_weapons` is scoped KILL-side by the server (its own comment is
   * explicit that weapons which killed YOU are a different metric), so this
   * reads as "what you kill with", never "what killed you".
   */
  topWeapons: StatsBucket[];
  deathsByZone: StatsBucket[];
}

export const combatMissionWidget = defineWidget<CombatMissionData>({
  id: 'combat_mission',
  eyebrow: 'Combat & Missions',
  rangeAware: true,
  visibility: 'owner',
  async load(ctx) {
    // Owner-only (see visibility). Defensive: never fetch me-scoped
    // metrics with a visitor's token even if load is reached directly.
    if (!ctx.isOwner || !ctx.token) return null;
    const token = ctx.token;
    // Per-type combat metrics + the newer mission_objective outcomes.
    // BOTH halves must share the selected window: objectives used to be
    // fetched unscoped, so a lifetime completion % rendered beside a
    // range-scoped combat breakdown under one range label. (The metrics
    // endpoint has no '24h' bucket, so a '24h' pick still widens that
    // half to 7d — see rangeToMetricsRange.)
    const hours = rangeToHours(ctx.range);
    const [breakdownRes, objectivesRes, combatRes] = await Promise.allSettled([
      getMetricsEventTypes(token, rangeToMetricsRange(ctx.range)),
      getObjectives(token, hours),
      // Same window as the other two: a lifetime weapon board beside a
      // range-scoped death count under one range label is the exact fault
      // the objectives half was already fixed for.
      getCombatStats(token, hours),
    ]);
    if (breakdownRes.status === 'rejected') {
      logger.warn({ err: breakdownRes.reason, call: 'widget.combat_mission' }, 'fetch failed');
    }
    if (objectivesRes.status === 'rejected') {
      logger.warn(
        { err: objectivesRes.reason, call: 'widget.combat_mission.objectives' },
        'fetch failed',
      );
    }
    if (combatRes.status === 'rejected') {
      logger.warn(
        { err: combatRes.reason, call: 'widget.combat_mission.combat' },
        'fetch failed',
      );
    }
    const combat = combatRes.status === 'fulfilled' ? combatRes.value : null;
    const breakdown = breakdownRes.status === 'fulfilled' ? breakdownRes.value : null;
    const objectives = objectivesRes.status === 'fulfilled' ? objectivesRes.value : null;
    if (!breakdown) return null;

    const counts = countsByType(breakdown.types);
    // Server-computed when we have it: it is the only source that can tell a
    // kill from a death, because that distinction lives in the payload rather
    // than in the event type.
    const deaths = combat?.deaths ?? sumCounts(breakdown.types, DEATH_TYPES_FALLBACK);
    const kills = combat?.kills ?? null;
    const incapacitated = sumCounts(breakdown.types, INCAPACITATED_TYPES);
    const vehicleLosses = sumCounts(breakdown.types, VEHICLE_LOSS_TYPES);
    const missionsStarted = sumCounts(breakdown.types, MISSION_START_TYPES);
    const missionsEnded = sumCounts(breakdown.types, MISSION_END_TYPES);
    const completionPct =
      missionsStarted > 0 ? Math.round((missionsEnded / missionsStarted) * 100) : null;
    const objectivePct = objectives?.completion_pct ?? null;

    // Empty when there's no combat/mission activity AND no objectives.
    if (
      deaths + vehicleLosses + missionsStarted === 0 &&
      !(objectives && objectives.total > 0)
    ) {
      return null;
    }

    return {
      deaths,
      kills,
      incapacitated,
      vehicleLosses,
      missionsStarted,
      missionsEnded,
      completionPct,
      objectivePct,
      counts,
      objectives,
      topWeapons: combat?.top_weapons ?? [],
      deathsByZone: combat?.deaths_by_zone ?? [],
    };
  },
  body(data, _ctx, size) {
    const {
      deaths,
      kills,
      vehicleLosses,
      missionsStarted,
      missionsEnded,
      completionPct,
      objectivePct,
      counts,
      objectives,
    } = data;

    if (size === 'compact') {
      const readouts: Readout[] = [
        ...(kills != null
          ? [{ label: 'kills', value: fmtNum(kills) } as Readout]
          : []),
        { label: 'deaths', value: fmtNum(deaths) },
        { label: 'veh loss', value: fmtNum(vehicleLosses) },
        { label: 'missions', value: fmtNum(missionsStarted) },
        ...(objectivePct != null
          ? [{ label: 'obj done', value: `${objectivePct}%` } as Readout]
          : []),
      ];
      return (
        <ReadoutGroup
          readouts={readouts}
          note={
            completionPct != null ? `${completionPct}% of missions completed` : undefined
          }
        />
      );
    }

    const rows: Row[] = [
      { key: 'player_death', label: 'Player deaths', value: fmtNum(counts['player_death'] ?? 0) },
      {
        key: 'player_incapacitated',
        label: 'Incapacitations',
        value: fmtNum(counts['player_incapacitated'] ?? 0),
      },
      { key: 'vehicle_losses', label: 'Vehicle losses', value: fmtNum(vehicleLosses) },
      { key: 'missions_started', label: 'Missions started', value: fmtNum(missionsStarted) },
      { key: 'missions_completed', label: 'Missions completed', value: fmtNum(missionsEnded) },
      ...(objectives && objectives.total > 0
        ? [
            {
              key: 'objectives_completed',
              label: 'Objectives completed',
              value: fmtNum(objectives.completed),
            },
            { key: 'objectives_failed', label: 'Objectives failed', value: fmtNum(objectives.failed) },
          ]
        : []),
    ];
    return <RankedList rows={rows} />;
  },
});
