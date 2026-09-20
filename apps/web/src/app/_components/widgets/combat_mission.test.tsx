import React from 'react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render } from '@testing-library/react';

vi.mock('@/lib/api', () => ({
  getMetricsEventTypes: vi.fn(),
  getObjectives: vi.fn(),
  // `top_weapons` and `deaths_by_zone` come from here. They were already on
  // this response and discarded; the widget now reads them, so the mock has
  // to offer the call or every test in this file fails on the mock rather
  // than on the widget.
  getCombatStats: vi.fn(),
}));

import { getCombatStats, getMetricsEventTypes, getObjectives } from '@/lib/api';
import { combatMissionWidget } from './combat_mission';
import { DEFAULT_SHARE_SCOPES } from './types';
import type { ViewerCtx } from './types';

function ownerCtx(range: ViewerCtx['range']): ViewerCtx {
  return {
    ownerHandle: 'alice',
    viewerHandle: 'alice',
    isOwner: true,
    token: 'tok',
    shareScopes: { ...DEFAULT_SHARE_SCOPES },
    recipientScopes: null,
    range,
  };
}

describe('combatMissionWidget range-awareness', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('is marked range-aware', () => {
    expect(combatMissionWidget.rangeAware).toBe(true);
  });

  it('passes ctx.range to getMetricsEventTypes', async () => {
    (getMetricsEventTypes as ReturnType<typeof vi.fn>).mockResolvedValue({
      types: [{ event_type: 'player_death', count: 3 }],
    });

    await combatMissionWidget.render(ownerCtx('90d'), 'compact');

    expect(getMetricsEventTypes).toHaveBeenCalledWith('tok', '90d');
  });

  it('sends a real 24h window now the endpoint serves that bucket', async () => {
    // Previously asserted 24h -> 7d, which encoded a server limitation
    // as intent: the widget rendered a WEEK under a "24h" label. The
    // endpoint gained a 24h bucket, so widening would now be the bug.
    (getMetricsEventTypes as ReturnType<typeof vi.fn>).mockResolvedValue({
      types: [{ event_type: 'player_death', count: 1 }],
    });

    await combatMissionWidget.render(ownerCtx('24h'), 'compact');

    expect(getMetricsEventTypes).toHaveBeenCalledWith('tok', '24h');
  });

  it('renders non-null when snake_case event types match', async () => {
    (getMetricsEventTypes as ReturnType<typeof vi.fn>).mockResolvedValue({
      types: [
        { event_type: 'player_death', count: 3 },
        { event_type: 'player_incapacitated', count: 1 },
        { event_type: 'actor_death', count: 2 },
        { event_type: 'mission_start', count: 5 },
        { event_type: 'mission_end', count: 4 },
      ],
    });

    const result = await combatMissionWidget.render(ownerCtx('90d'), 'compact');

    expect(result).not.toBeNull();
  });

  it('surfaces objective completion % from getObjectives', async () => {
    (getMetricsEventTypes as ReturnType<typeof vi.fn>).mockResolvedValue({
      types: [{ event_type: 'mission_start', count: 4 }],
    });
    (getObjectives as ReturnType<typeof vi.fn>).mockResolvedValue({
      completed: 3,
      failed: 1,
      in_progress: 1,
      unresolved: 0,
      total: 5,
      completion_pct: 75,
    });

    const node = await combatMissionWidget.render(ownerCtx('90d'), 'compact');
    const { container } = render(node as React.ReactElement);

    // 90d => 24*90 = 2160 hours. Objectives MUST be range-scoped, not lifetime.
    expect(getObjectives).toHaveBeenCalledWith('tok', 2160);
    expect(container.textContent).toContain('75%');
    expect(container.textContent).toContain('obj done');
  });

  it('passes the ctx.range window (hours) to getObjectives', async () => {
    (getMetricsEventTypes as ReturnType<typeof vi.fn>).mockResolvedValue({
      types: [{ event_type: 'mission_start', count: 4 }],
    });
    (getObjectives as ReturnType<typeof vi.fn>).mockResolvedValue({
      completed: 3,
      failed: 1,
      in_progress: 0,
      unresolved: 0,
      total: 4,
      completion_pct: 75,
    });

    await combatMissionWidget.render(ownerCtx('30d'), 'compact');

    // 30d => 24*30 = 720 hours, passed as the 2nd arg.
    expect(getObjectives).toHaveBeenCalledWith('tok', 720);
  });

  it('never fetches objectives unscoped (lifetime) while metrics are range-scoped', async () => {
    // Regression guard: `getObjectives(token)` with no hours returned
    // all-time totals, so the tile rendered a 30-day combat breakdown
    // beside a lifetime objective % under one range label.
    (getMetricsEventTypes as ReturnType<typeof vi.fn>).mockResolvedValue({
      types: [{ event_type: 'mission_start', count: 4 }],
    });
    (getObjectives as ReturnType<typeof vi.fn>).mockResolvedValue({
      completed: 1,
      failed: 0,
      in_progress: 0,
      unresolved: 0,
      total: 1,
      completion_pct: 100,
    });

    await combatMissionWidget.render(ownerCtx('7d'), 'compact');

    const objectivesArgs = (getObjectives as ReturnType<typeof vi.fn>).mock.calls[0];
    expect(objectivesArgs).toHaveLength(2);
    expect(objectivesArgs[1]).toBe(168);
    expect(objectivesArgs[1]).not.toBeUndefined();
  });
});

describe('combatMissionWidget C2 owner-only gating', () => {
  const visitorCtx: ViewerCtx = {
    ownerHandle: 'alice',
    viewerHandle: 'bob',
    isOwner: false,
    token: 'bob-tok',
    shareScopes: { ...DEFAULT_SHARE_SCOPES, combat_mission: true },
    recipientScopes: null,
    range: '7d',
  };

  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('is available to the owner', () => {
    expect(combatMissionWidget.isAvailable(ownerCtx('7d'))).toBe(true);
  });

  it('is UNavailable to a visitor even with the combat_mission share scope on', () => {
    // No friend-scoped metrics endpoint exists, so the widget must not
    // render for a visitor — it would surface the viewer's own data.
    expect(combatMissionWidget.isAvailable(visitorCtx)).toBe(false);
  });

  it('render returns null for a visitor without calling the me endpoint', async () => {
    const result = await combatMissionWidget.render(visitorCtx, 'compact');
    expect(result).toBeNull();
    expect(getMetricsEventTypes).not.toHaveBeenCalled();
  });
});

describe('combatMissionWidget death accounting', () => {
  /**
   * A KILL IS NOT A DEATH, and this widget used to count it as one.
   *
   * `actor_death` fires whether the caller was the victim or the killer, so
   * summing the event type added every kill to the death total.
   * `player_incapacitated` — downed but alive — was in the same sum. Only the
   * server can separate them, because the distinction lives in the payload:
   * `/v1/me/stats/combat` filters on whether the caller is the killer or the
   * victim, and unions `player_death` for modern builds.
   *
   * The fixture is the shape that exposed it: 21 `actor_death` rows that the
   * server scores as 21 kills and 12 deaths. The widget displayed 21.
   */
  it('reports the server death count, not the actor_death event count', async () => {
    vi.mocked(getMetricsEventTypes).mockResolvedValue({
      types: [
        { event_type: 'actor_death', count: 21 },
        { event_type: 'player_incapacitated', count: 5 },
        { event_type: 'vehicle_destruction', count: 4 },
      ],
    } as never);
    vi.mocked(getObjectives).mockResolvedValue(null as never);
    vi.mocked(getCombatStats).mockResolvedValue({
      hours: 168,
      kills: 21,
      deaths: 12,
      top_weapons: [],
      deaths_by_zone: [],
    } as never);

    const data = (await combatMissionWidget.load!(ownerCtx('7d'))) as {
      deaths: number;
      incapacitated: number;
    } | null;
    expect(data).not.toBeNull();
    expect(data!.deaths, 'kills must not be added to deaths').toBe(12);
    // The `kills` assertion that stood here is gone with the field: nothing
    // can supply a kill (see the note in combat_mission.tsx). What this test
    // is really for survives intact — 21 actor_death rows must not become 21
    // deaths.
    // Downed is its own outcome, never folded into deaths.
    expect(data!.incapacitated).toBe(5);
  });

  /**
   * The kill figure is back, and it is named for what it counts.
   *
   * It was removed in 6a7184e on the reasoning that nothing could supply a
   * kill: `ACTOR_DEATH_RE` sits under a comment calling the combat patterns
   * "derived from community captures, NOT this fixture", and its only test is
   * named `classifies_synthetic_actor_death`. That reasoning was wrong about
   * the conclusion while being right about the fixture. This machine's logs
   * genuinely contain no combat — 314 files, zero `<Actor Death>` lines, which
   * is why the fixture test had to be synthetic — but production holds
   * thousands of `actor_death` rows with populated `killer` and `victim`,
   * and the regex is all-or-nothing: it cannot match without also capturing
   * `weapon`, `zone` and `damage type`. The parser works; the capture was
   * missing.
   *
   * What it is NOT is a count of players killed. CIG stopped writing a line
   * when one player kills another, so the label says NPC and the enemy board
   * below carries the evidence for that claim.
   */
  it('reports NPC kills and names the enemy rather than the engine id', async () => {
    vi.mocked(getMetricsEventTypes).mockResolvedValue({
      types: [{ event_type: 'actor_death', count: 21 }],
    } as never);
    vi.mocked(getObjectives).mockResolvedValue(null as never);
    vi.mocked(getCombatStats).mockResolvedValue({
      hours: 168,
      kills: 2120,
      deaths: 12,
      top_weapons: [{ value: 'behr_rifle_ballistic_01', count: 1074 }],
      top_damage_types: [{ value: 'Bullet', count: 2120 }],
      top_enemies: [
        {
          display: 'Human Criminal Pilot Light',
          group_key: 'PU_Pilots-Human-Criminal-Pilot_Light',
          family: 'human',
          count: 1074,
        },
        {
          display: 'Kopion Irradiated',
          group_key: 'Kopion_Irradiated',
          family: 'creature',
          count: 1046,
        },
      ],
      deaths_by_zone: [],
    } as never);

    const data = (await combatMissionWidget.load!(ownerCtx('7d'))) as {
      npcKills: number | null;
      topEnemies: { display: string; family: string; count: number }[];
    } | null;
    expect(data).not.toBeNull();
    expect(data!.npcKills).toBe(2120);
    expect(data!.topEnemies).toHaveLength(2);

    const html = render(
      <>{await combatMissionWidget.render(ownerCtx('7d'), 'expanded')}</>,
    ).container.textContent;

    expect(html, 'the count has to be labelled for what it counts').toContain(
      'NPC kills',
    );
    expect(html).toContain('Human Criminal Pilot Light');
    expect(
      html,
      'the raw engine identifier is not a name a reader recognises',
    ).not.toContain('PU_Pilots');
  });

  /**
   * A FAILED combat call must not render as "0 NPC kills".
   *
   * This is the reason the field was nullable before it was deleted, and the
   * reason 6a7184e gave for deleting it rather than promoting it to a always-
   * rendered row: a zero ASSERTS you killed nothing, where an absence says
   * nothing. That argument was right, and survives the field's return.
   */
  it('omits the kill readout entirely when the combat call failed', async () => {
    vi.mocked(getMetricsEventTypes).mockResolvedValue({
      types: [{ event_type: 'actor_death', count: 6 }],
    } as never);
    vi.mocked(getObjectives).mockResolvedValue(null as never);
    vi.mocked(getCombatStats).mockRejectedValue(new Error('boom'));

    const data = (await combatMissionWidget.load!(ownerCtx('7d'))) as {
      npcKills: number | null;
    } | null;
    expect(data!.npcKills).toBeNull();

    const html = render(
      <>{await combatMissionWidget.render(ownerCtx('7d'), 'expanded')}</>,
    ).container.textContent;
    // NOT a vacuous pass: the tile still rendered, it simply has no kill
    // row. Without this the assertion above would also hold for a render
    // that returned null and produced an empty string.
    expect(html).toContain('Player deaths');
    expect(html).not.toContain('NPC kills');
  });

  /**
   * A hull lost through the only line that still reports it.
   *
   * `vehicle_destruction` holds ZERO rows on a 320,945-event database — CIG
   * stopped writing `<Vehicle Destruction>` in modern builds, as it did
   * `<Actor Death>`. The `[ActorState] Dead` line is what survives: it fires
   * when you are ejected from a vehicle destroyed around you and it names the
   * ship. It appeared 371 times on that database while "Hulls lost" read 0.
   */
  it('counts an ejection from a destroyed vehicle as a lost hull', async () => {
    vi.mocked(getMetricsEventTypes).mockResolvedValue({
      types: [
        { event_type: 'actor_ejected', count: 7 },
        { event_type: 'mission_start', count: 3 },
      ],
    } as never);
    vi.mocked(getObjectives).mockResolvedValue(null as never);
    vi.mocked(getCombatStats).mockRejectedValue(new Error('no combat call'));

    const data = (await combatMissionWidget.load!(ownerCtx('7d'))) as {
      vehicleLosses: number;
    } | null;
    expect(
      data!.vehicleLosses,
      'the ship is gone whichever line the engine chose to report it with',
    ).toBe(7);
  });

  /**
   * A metric with NO events behind it must not render as 0.
   *
   * `mission_start` holds zero rows on that same database while `mission_end`
   * holds 1,238, so the tile read "Contracts started 0 / Contracts ended
   * 1,238". That is not a low number, it is a missing one — and a 0 asserts
   * you started nothing, which is the same fault the kill count was deleted
   * over. A handle that has genuinely started none still carries the type at
   * count 0, so a real zero survives.
   */
  it('omits missions started when nothing has ever reported one', async () => {
    vi.mocked(getMetricsEventTypes).mockResolvedValue({
      types: [{ event_type: 'mission_end', count: 1238 }],
    } as never);
    vi.mocked(getObjectives).mockResolvedValue(null as never);
    vi.mocked(getCombatStats).mockRejectedValue(new Error('no combat call'));

    const data = (await combatMissionWidget.load!(ownerCtx('7d'))) as {
      missionsStarted: number | null;
      completionPct: number | null;
    } | null;
    expect(data!.missionsStarted).toBeNull();
    // And nothing divides by it.
    expect(data!.completionPct).toBeNull();

    const html = render(
      <>{await combatMissionWidget.render(ownerCtx('7d'), 'expanded')}</>,
    ).container.textContent;
    expect(html).not.toContain('Missions started');
    // Not a vacuous pass - the tile did render.
    expect(html).toContain('Missions completed');
  });

  /**
   * NOTE: this fixture is SYNTHETIC in a way production is not.
   *
   * `/v1/me/metrics/event-types` is windowed and omits a type with no rows in
   * the window — it never returns one with `count: 0` (see the note on
   * `event_type_breakdown`). So the "type present with a zero count" case
   * below cannot currently arrive from the real API.
   *
   * The test is kept because the BRANCH is what matters: if a type is ever
   * reported present-and-zero, it must render as 0 rather than vanish. It
   * documents intent, not observed behaviour, and is labelled so nobody reads
   * it as proof the endpoint does this.
   */
  it('keeps a REAL zero when the type has reported before', async () => {
    vi.mocked(getMetricsEventTypes).mockResolvedValue({
      types: [
        { event_type: 'mission_start', count: 0 },
        { event_type: 'mission_end', count: 4 },
      ],
    } as never);
    vi.mocked(getObjectives).mockResolvedValue(null as never);
    vi.mocked(getCombatStats).mockRejectedValue(new Error('no combat call'));

    const data = (await combatMissionWidget.load!(ownerCtx('7d'))) as {
      missionsStarted: number | null;
    } | null;
    expect(
      data!.missionsStarted,
      'a genuine zero is an answer and must survive',
    ).toBe(0);
  });

  /**
   * A metric the game no longer emits must not render as 0.
   *
   * The newest `actor_death` row in the entire database is 2025-11-19 — 306
   * days before this was written, with zero rows in 2026 across every handle.
   * CIG stopped writing the line. So for almost every reader "NPC kills 0"
   * asserts they killed nothing, when the truth is that nothing can report a
   * kill; that is the same fault this widget's own history is littered with.
   */
  it('omits NPC kills when the window holds no kill events', async () => {
    vi.mocked(getMetricsEventTypes).mockResolvedValue({
      types: [{ event_type: 'player_death', count: 9 }],
    } as never);
    vi.mocked(getObjectives).mockResolvedValue(null as never);
    vi.mocked(getCombatStats).mockResolvedValue({
      hours: 168,
      kills: 0,
      deaths: 9,
      top_weapons: [],
      top_damage_types: [],
      top_enemies: [],
      deaths_by_zone: [],
    } as never);

    const data = (await combatMissionWidget.load!(ownerCtx('7d'))) as {
      npcKills: number | null;
    } | null;
    expect(data!.npcKills).toBeNull();

    const html = render(
      <>{await combatMissionWidget.render(ownerCtx('7d'), 'expanded')}</>,
    ).container.textContent;
    expect(html).not.toContain('NPC kills');
    // Not vacuous - the tile rendered.
    expect(html).toContain('Player deaths');
  });

  /**
   * The accounts that DO have kill history keep the figure.
   *
   * Four handles hold 49,541 kill rows between them. Omitting the metric
   * outright would throw their history away to spare everyone else a zero.
   */
  it('still reports kills for a window that has them', async () => {
    vi.mocked(getMetricsEventTypes).mockResolvedValue({
      types: [{ event_type: 'actor_death', count: 21 }],
    } as never);
    vi.mocked(getObjectives).mockResolvedValue(null as never);
    vi.mocked(getCombatStats).mockResolvedValue({
      hours: 168,
      kills: 19,
      deaths: 2,
      top_weapons: [],
      top_damage_types: [],
      top_enemies: [],
      deaths_by_zone: [],
    } as never);

    const data = (await combatMissionWidget.load!(ownerCtx('7d'))) as {
      npcKills: number | null;
    } | null;
    expect(data!.npcKills).toBe(19);
  });

  it('falls back to the event count when the combat call fails', async () => {
    // An over-count beats a blank when the authoritative source is down —
    // but incapacitation stays out of it either way.
    vi.mocked(getMetricsEventTypes).mockResolvedValue({
      types: [
        { event_type: 'actor_death', count: 6 },
        { event_type: 'player_death', count: 2 },
        { event_type: 'player_incapacitated', count: 4 },
      ],
    } as never);
    vi.mocked(getObjectives).mockResolvedValue(null as never);
    vi.mocked(getCombatStats).mockRejectedValue(new Error('boom'));

    const data = (await combatMissionWidget.load!(ownerCtx('7d'))) as {
      deaths: number;
      incapacitated: number;
    } | null;
    expect(data).not.toBeNull();
    expect(data!.deaths).toBe(8);
    expect(data!.incapacitated).toBe(4);
  });
});
