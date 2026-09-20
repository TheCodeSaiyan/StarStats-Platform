import React from 'react';
import { getCombatStats, getMetricsEventTypes, getObjectives } from '@/lib/api';
import type { EnemyBucket, ObjectivesResponse, StatsBucket } from '@/lib/api';
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
/**
 * Both sources of a lost hull.
 *
 * `vehicle_destruction` holds ZERO rows on a 320,945-event database — CIG
 * appears to have stopped writing `<Vehicle Destruction>` in modern builds,
 * exactly as it did `<Actor Death>`. `actor_ejected` is what survives: the
 * `[ActorState] Dead` line that fires when you are thrown out of a vehicle
 * destroyed around you, which names the ship. It appeared 371 times on that
 * same database while "Hulls lost" read 0.
 *
 * Both are counted because the old type is still right for historical rows.
 */
const VEHICLE_LOSS_TYPES = ['vehicle_destruction', 'actor_ejected'];
const MISSION_START_TYPES = ['mission_start'];
const MISSION_END_TYPES = ['mission_end'];

interface CombatMissionData {
  deaths: number;
  /**
   * Kills the log recorded. `null` when the combat call FAILED.
   *
   * Null rather than 0 on failure, and omitted from the render when null: a
   * zero asserts you killed nothing, where an absence says nothing. That was
   * the argument for deleting this field in 6a7184e and it is still right —
   * what was wrong there was the premise, not the caution.
   *
   * The premise was that nothing can supply a kill, because `ACTOR_DEATH_RE`
   * is only covered by a test named `classifies_synthetic_actor_death`. The
   * fixture really is combat-free — this machine's 314 log files hold zero
   * `<Actor Death>` lines — but production holds thousands of `actor_death`
   * rows with populated `killer` and `victim`, and the regex is all-or-
   * nothing: it cannot match without also capturing weapon, zone and damage
   * type. The parser was working the whole time; the CAPTURE was missing.
   *
   * Named for NPCs because that is what it counts. CIG no longer writes a log
   * line when one player kills another, so a kill that reaches us is PvE.
   * `topEnemies` carries the evidence — if a `player_like` family ever shows
   * up there, this name has stopped being true.
   */
  npcKills: number | null;
  /** What was killed, grouped by archetype and already humanised server-side. */
  topEnemies: EnemyBucket[];
  /** Damage types DEALT — same kill-side scoping as `topWeapons`. */
  topDamageTypes: StatsBucket[];
  /** Downed but not killed — never folded into `deaths`, which is what this
   *  widget used to do. */
  incapacitated: number;
  vehicleLosses: number;
  missionsStarted: number | null;
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
    const incapacitated = sumCounts(breakdown.types, INCAPACITATED_TYPES);
    const vehicleLosses = sumCounts(breakdown.types, VEHICLE_LOSS_TYPES);
    // `null`, not 0, when the event type is absent from the breakdown
    // ENTIRELY. `mission_start` holds zero rows on a 320,945-event database
    // while `mission_end` holds 1,238, so the tile rendered "Contracts
    // started 0 / Contracts ended 1,238" — which is not a low number, it is
    // a missing one, and a 0 asserts you started nothing.
    //
    // A handle that genuinely started no contracts in the window still has
    // the type present with a count of 0 once it has ever fired, so a real
    // zero is preserved.
    // `counts` is the record; `breakdown.types` is the ARRAY it came from.
    // Presence has to be asked of the record — `t in array` tests INDICES.
    const missionsStarted = MISSION_START_TYPES.some((t) => t in counts)
      ? sumCounts(breakdown.types, MISSION_START_TYPES)
      : null;
    const missionsEnded = sumCounts(breakdown.types, MISSION_END_TYPES);
    const completionPct =
      missionsStarted != null && missionsStarted > 0
        ? Math.round((missionsEnded / missionsStarted) * 100)
        : null;
    const objectivePct = objectives?.completion_pct ?? null;

    // Empty when there is no combat/mission activity AND no objectives.
    //
    // `missionsEnded` and `incapacitated` are in the sum deliberately. They
    // were not, and `mission_start` reports nothing on modern builds — so an
    // account with 1,238 completed contracts and no deaths in the window had
    // the whole tile disappear, which reads as "you did nothing" rather than
    // "one of these five numbers is unavailable".
    if (
      deaths +
        vehicleLosses +
        incapacitated +
        missionsEnded +
        (missionsStarted ?? 0) ===
        0 &&
      !(objectives && objectives.total > 0)
    ) {
      return null;
    }

    return {
      deaths,
      incapacitated,
      vehicleLosses,
      missionsStarted,
      missionsEnded,
      completionPct,
      objectivePct,
      counts,
      objectives,
      // `?? null`, never `?? 0`: see the field's note. `combat` is undefined
      // when the call rejected, and 0 is a different claim from "unknown".
      npcKills: combat?.kills ?? null,
      topEnemies: combat?.top_enemies ?? [],
      topDamageTypes: combat?.top_damage_types ?? [],
      topWeapons: combat?.top_weapons ?? [],
      deathsByZone: combat?.deaths_by_zone ?? [],
    };
  },
  body(data, _ctx, size) {
    const {
      deaths,
      npcKills,
      topEnemies,
      topWeapons,
      topDamageTypes,
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
        ...(npcKills != null
          ? [{ label: 'npc kills', value: fmtNum(npcKills) } as Readout]
          : []),
        { label: 'deaths', value: fmtNum(deaths) },
        { label: 'veh loss', value: fmtNum(vehicleLosses) },
        ...(missionsStarted != null
          ? [{ label: 'missions', value: fmtNum(missionsStarted) } as Readout]
          : []),
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
      // Omitted entirely when null. A failed read must not render as a zero.
      ...(npcKills != null
        ? [{ key: 'npc_kills', label: 'NPC kills', value: fmtNum(npcKills) }]
        : []),
      { key: 'player_death', label: 'Player deaths', value: fmtNum(counts['player_death'] ?? 0) },
      {
        key: 'player_incapacitated',
        label: 'Incapacitations',
        value: fmtNum(counts['player_incapacitated'] ?? 0),
      },
      { key: 'vehicle_losses', label: 'Vehicle losses', value: fmtNum(vehicleLosses) },
      ...(missionsStarted != null
        ? [{ key: 'missions_started', label: 'Missions started', value: fmtNum(missionsStarted) }]
        : []),
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
    // The boards beneath the counts. Each is capped at five: the response
    // carries up to STATS_BUCKET_LIMIT (100) and a tile is not a report.
    //
    // `display` is what the server humanised; `group_key` — the engine name
    // with its entity id stripped — rides along in the title so an odd label
    // can be traced back to what was actually in the log rather than argued
    // about. An `unclassified` family is shown, not hidden: a board that drops
    // what it could not parse misreports its own total.
    const boards: { key: string; heading: string; rows: Row[] }[] = [
      {
        key: 'enemies',
        heading: 'Most killed',
        rows: topEnemies.slice(0, 5).map((e) => ({
          key: `enemy:${e.group_key}`,
          label: (
            <span title={e.group_key}>
              {e.display}
              {e.family === 'unclassified' ? ' (unrecognised)' : ''}
            </span>
          ),
          value: fmtNum(e.count),
        })),
      },
      {
        key: 'weapons',
        heading: 'Killed with',
        rows: topWeapons.slice(0, 5).map((w) => ({
          key: `weapon:${w.value}`,
          label: w.value,
          value: fmtNum(w.count),
        })),
      },
      {
        key: 'damage',
        heading: 'Damage dealt',
        rows: topDamageTypes.slice(0, 5).map((d) => ({
          key: `damage:${d.value}`,
          label: d.value,
          value: fmtNum(d.count),
        })),
      },
    ].filter((b) => b.rows.length > 0);

    return (
      <div className="hud-readout-stack">
        <RankedList rows={rows} />
        {boards.map((b) => (
          <div key={b.key}>
            <p className="hud-tile__eyebrow">{b.heading}</p>
            <RankedList rows={b.rows} />
          </div>
        ))}
      </div>
    );
  },
});
