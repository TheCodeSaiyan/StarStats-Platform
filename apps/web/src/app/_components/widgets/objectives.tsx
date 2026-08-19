import React from 'react';
import { getObjectives } from '@/lib/api';
import { logger } from '@/lib/logger';
import { rangeToWindowHours, rangeHasLifetimeBaseline } from '@/lib/range';
import { computeTrend, formatTrend, previousWindowLabel } from '@/lib/trend';
import { EmptyWindow } from './kit/EmptyWindow';
import { defineWidget } from './kit/defineWidget';
import { ReadoutGroup } from './kit/archetypes';
import { fmtNum, fmtPct } from './kit/format';
import { InfoTip } from '@/components/hud/InfoTip';
import { INFERENCE_EXPLANATIONS } from '@/lib/inference-explanations';

/**
 * `objectives` — distinct-objective completion from the reparse-gated
 * `GET /v1/me/stats/objectives` aggregate (`mission_objective` events):
 * completed / failed / unresolved / no-outcome plus a completion-rate
 * meter. `unresolved` is resolved-but-not-completed: withdrawn
 * objectives, plus any terminal state CIG hasn't given the parser a
 * mapping for yet — surfaced explicitly rather than folded into the
 * headline silently, so the completion rate stays honest. `no_outcome` is
 * objectives for which no terminal state was EVER observed — NOT
 * objectives the player currently has active (a player holds a handful
 * at most; on real accounts this bucket can run into the hundreds, from
 * abandoned missions, app exits, and log rotations mid-mission — it was
 * mislabelled `in_progress` before, which read as a live count and
 * wasn't one).
 * Owner-only (me-scoped, no friend equivalent), range-aware (follows the
 * dashboard range selector).
 *
 * Migrated to the kit: `ReadoutGroup` carries the done/rate headline and
 * the secondary no-outcome/unresolved/failed breakdown (collapses when
 * the tile is squeezed). The single completion bar is a bespoke
 * `hud-meter` — a one-bar case that `MeterList` (labelled multi-bar)
 * doesn't model, so it stays inline per the kit guidance.
 *
 * The headline `done <completed>/<resolved>` and the `rate` meter share
 * the SAME denominator (`completed + failed + unresolved`) so the two
 * figures on the tile always agree — `resolved` deliberately excludes
 * `no_outcome`, which never resolved either way. Do not swap it back to
 * the API's `total` (which includes `no_outcome`): that reintroduces the
 * "done 180/1225 next to rate 64%" mismatch this widget used to show.
 */
interface ObjectivesData {
  completed: number;
  /** completed + failed + unresolved — the SAME denominator `pct` uses. */
  resolved: number;
  noOutcome: number;
  unresolved: number;
  failed: number;
  /** 0–100 completion percentage of RESOLVED objectives. */
  pct: number;
  /** Lifetime baseline the windowed headline is read against. Null on the
   *  `all` range — it spans the full 365 days of retention, so its twin
   *  covers the same rows and the note would restate the headline rather
   *  than contextualise it — and null whenever the server sent no twin.
   *  Never substituted or estimated: a fabricated baseline is worse than
   *  a bare number.
   *
   *  `resolved` is derived on the SAME completed-over-resolved basis as the
   *  windowed `resolved` above, so the two ratios are comparable at a glance;
   *  `pct` stays null when the career rate is genuinely undefined. */
  lifetime: { completed: number; resolved: number; pct: number | null } | null;
  /** Same-length window immediately before this one. Only `completed` is
   *  carried: it is the headline figure, and a trend on the rate would
   *  compare two ratios whose denominators differ, which reads as a
   *  change in performance when it can be a change in volume. Null means
   *  "no comparison to draw" — never coerce to 0. */
  previous: { completed: number } | null;
  /** Lifetime objective count, used ONLY to distinguish "nothing in this
   *  window" from "nothing ever" when the window is empty. Separate from
   *  `lifetime` above, which is null on the `all` range. */
  lifetimeTotal: number;
}

export const objectivesWidget = defineWidget<ObjectivesData>({
  id: 'objectives',
  eyebrow: 'Missions',
  rangeAware: true,
  visibility: 'owner',
  async load(ctx) {
    if (!ctx.token) return null;
    let obj = null;
    try {
      obj = await getObjectives(ctx.token, rangeToWindowHours(ctx.range));
    } catch (err) {
      logger.warn({ err, call: 'widget.objectives' }, 'fetch failed');
      return null;
    }
    // An empty WINDOW is not an empty account — see kit/EmptyWindow.
    const lifetimeTotal = rangeHasLifetimeBaseline(ctx.range)
      ? (obj?.lifetime?.total ?? 0)
      : 0;
    if (!obj || (obj.total === 0 && lifetimeTotal === 0)) return null;
    const unresolved = obj.unresolved ?? 0;
    const resolved = obj.completed + obj.failed + unresolved;
    const pct =
      obj.completion_pct != null
        ? obj.completion_pct
        : Math.round((obj.completed / Math.max(resolved, 1)) * 100);
    const lt = obj.lifetime;
    return {
      completed: obj.completed,
      resolved,
      noOutcome: obj.no_outcome,
      unresolved,
      failed: obj.failed,
      pct,
      // Dropped on `all`: that range already spans retention, so the
      // twin covers the same rows and the note would read "N of N".
      lifetime:
        lt && rangeHasLifetimeBaseline(ctx.range)
        ? {
            completed: lt.completed,
            resolved: lt.completed + lt.failed + (lt.unresolved ?? 0),
            // Null means nothing has EVER resolved. Computing it locally
            // would print "0%", which reads as a measured career rate
            // rather than the absence of one — so the rate is dropped from
            // the comparison instead.
            pct: lt.completion_pct ?? null,
          }
        : null,
      // Not range-gated: the server already omits `previous` for `all`
      // and for a handle with no prior activity.
      previous: obj.previous
        ? { completed: obj.previous.completed }
        : null,
      lifetimeTotal: lifetimeTotal,
    };
  },
  body(data, ctx) {
    if (data.resolved === 0 && data.noOutcome === 0) {
      return (
        <EmptyWindow
          rangeLabel={previousWindowLabel(ctx.range)}
          lifetimeCount={data.lifetimeTotal}
          noun="mission objectives"
        />
      );
    }
    // Baseline for the done/rate headline, hung off the LAST group so it
    // lands at the foot of the tile like every other widget's note rather
    // than splitting the headline from the meter that draws it.
    const lt = data.lifetime;
    const lifetimeNote = lt
      ? `Lifetime — ${fmtNum(lt.completed)}/${fmtNum(lt.resolved)} done${
          lt.pct != null ? `, ${fmtPct(lt.pct, true)}` : ''
        }`
      : undefined;
    // Trend on objectives COMPLETED — see `ObjectivesData.previous` for
    // why the rate is deliberately not trended.
    const trend = data.previous
      ? formatTrend(
          computeTrend(data.completed, data.previous.completed),
          previousWindowLabel(ctx.range),
          fmtNum,
        )
      : undefined;
    const note = trend ?? lifetimeNote;
    return (
      <div className="hud-readout-stack">
        <ReadoutGroup
          readouts={[
            { label: 'done', value: `${fmtNum(data.completed)}/${fmtNum(data.resolved)}` },
            { label: 'rate', value: fmtPct(data.pct, true) },
          ]}
        />
        <div className="hud-meter" aria-hidden="true">
          <span
            className="hud-meter__fill"
            style={{ ['--val' as string]: `${data.pct}%` } as React.CSSProperties}
          />
        </div>
        <ReadoutGroup
          readouts={[
            {
              label: 'no outcome',
              info: (
                <InfoTip
                  label="no outcome"
                  text={INFERENCE_EXPLANATIONS.objectives_no_outcome}
                />
              ),
              value: fmtNum(data.noOutcome),
              secondary: true,
            },
            { label: 'unresolved', value: fmtNum(data.unresolved), secondary: true },
            { label: 'failed', value: fmtNum(data.failed), secondary: true },
          ]}
          note={note}
        />
      </div>
    );
  },
});
