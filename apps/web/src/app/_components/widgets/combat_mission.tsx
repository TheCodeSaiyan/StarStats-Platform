import React from 'react';
import { getMetricsEventTypes, getObjectives } from '@/lib/api';
import type { ObjectivesResponse } from '@/lib/api';
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
const DEATH_TYPES = ['player_death', 'player_incapacitated', 'actor_death'];
const VEHICLE_LOSS_TYPES = ['vehicle_destruction'];
const MISSION_START_TYPES = ['mission_start'];
const MISSION_END_TYPES = ['mission_end'];

interface CombatMissionData {
  deaths: number;
  vehicleLosses: number;
  missionsStarted: number;
  missionsEnded: number;
  completionPct: number | null;
  objectivePct: number | null;
  counts: Record<string, number>;
  objectives: ObjectivesResponse | null;
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
    const [breakdownRes, objectivesRes] = await Promise.allSettled([
      getMetricsEventTypes(token, rangeToMetricsRange(ctx.range)),
      getObjectives(token, hours),
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
    const breakdown = breakdownRes.status === 'fulfilled' ? breakdownRes.value : null;
    const objectives = objectivesRes.status === 'fulfilled' ? objectivesRes.value : null;
    if (!breakdown) return null;

    const counts = countsByType(breakdown.types);
    const deaths = sumCounts(breakdown.types, DEATH_TYPES);
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
      vehicleLosses,
      missionsStarted,
      missionsEnded,
      completionPct,
      objectivePct,
      counts,
      objectives,
    };
  },
  body(data, _ctx, size) {
    const {
      deaths,
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
