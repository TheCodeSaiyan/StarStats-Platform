import React from 'react';
import { getDocking } from '@/lib/api';
import type { DockingResponse } from '@/lib/api';
import { logger } from '@/lib/logger';
import { rangeToWindowHours, rangeHasLifetimeBaseline } from '@/lib/range';
import { computeTrend, formatTrend, previousWindowLabel } from '@/lib/trend';
import { EmptyWindow } from './kit/EmptyWindow';
import { defineWidget } from './kit/defineWidget';
import { MeterList, type Readout } from './kit/archetypes';
import { fmtNum } from './kit/format';
import { InfoTip } from '@/components/hud/InfoTip';
import { INFERENCE_EXPLANATIONS } from '@/lib/inference-explanations';

/**
 * `docking` — "Where you dock": hangar-vs-pad split plus ship-size
 * distribution of stow events (`GET /v1/me/stats/docking`). Owner-only,
 * range-aware (follows the dashboard range selector).
 *
 * Migrated to the kit: `MeterList` owns the header readouts (kind split)
 * plus the share-of-total size bars. Numeric-only aggregate — no ship
 * identity, so (unlike `fleet`) no `<EntityLink>`. The "From ship stowing"
 * caveat rides as the list note.
 */
const SIZE_ROWS: Array<{ key: keyof DockingResponse['by_size']; label: string }> = [
  { key: 'small', label: 'Small' },
  { key: 'medium', label: 'Medium' },
  { key: 'large', label: 'Large' },
  { key: 'xl', label: 'XL' },
];

/** Provenance caveat — these are ship-stow events, not a dock turnstile. */
const STOW_CAVEAT = 'From ship stowing';

interface DockingData {
  by_kind: DockingResponse['by_kind'];
  by_size: DockingResponse['by_size'];
  total: number;
  /** Lifetime baseline for `total` (UX Rule 2 — a bare "10 stows" says
   *  nothing until you know it's 10 of 412). Null on the `all` range,
   *  which spans the whole of retention: the twin covers the same rows,
   *  so there is nothing to compare against and the note stays the bare
   *  caveat. */
  lifetime: NonNullable<DockingResponse['lifetime']> | null;
  /** Same-length window immediately before this one. Null means "no
   *  comparison to draw" — the server omits it when the handle had no
   *  activity at all back then (they were not a user yet) and on `all`,
   *  which has no predecessor inside retention. NEVER coerce to 0: a
   *  zero is a genuine "played but stowed nothing", which reads very
   *  differently from "we have nothing to compare". */
  previous: NonNullable<DockingResponse['previous']> | null;
}

export const dockingWidget = defineWidget<DockingData>({
  id: 'docking',
  eyebrow: 'Docking',
  rangeAware: true,
  visibility: 'owner',
  async load(ctx) {
    if (!ctx.token) return null;
    let docking: DockingResponse | null = null;
    try {
      docking = await getDocking(ctx.token, rangeToWindowHours(ctx.range));
    } catch (err) {
      logger.warn({ err, call: 'widget.docking' }, 'fetch failed');
      return null;
    }
    const total = docking?.total_stows ?? 0;
    // An empty WINDOW is not an empty account — see kit/EmptyWindow.
    // Only bail when there is nothing to report either way.
    const lifetimeStows = rangeHasLifetimeBaseline(ctx.range)
      ? (docking?.lifetime?.total_stows ?? 0)
      : 0;
    if (!docking || (total === 0 && lifetimeStows === 0)) return null;
    return {
      by_kind: docking.by_kind,
      by_size: docking.by_size,
      total,
      // Dropped on `all`: that range already spans retention, so the
      // twin covers the same rows and the note would read "N of N".
      lifetime: rangeHasLifetimeBaseline(ctx.range)
        ? (docking.lifetime ?? null)
        : null,
      previous: docking.previous ?? null,
    };
  },
  body(data, ctx) {
    if (data.total === 0) {
      return (
        <EmptyWindow
          rangeLabel={previousWindowLabel(ctx.range)}
          lifetimeCount={data.lifetime?.total_stows ?? 0}
          noun="ship stows"
        />
      );
    }
    const header: Readout[] = [
      {
        label: 'hangar',
        info: <InfoTip label="the hangar/pad split" text={INFERENCE_EXPLANATIONS.docking_kind} />,
        value: fmtNum(data.by_kind.hangar),
      },
      { label: 'pad', value: fmtNum(data.by_kind.pad) },
      { label: 'other', value: fmtNum(data.by_kind.other) },
    ];
    const meters = SIZE_ROWS.map(({ key, label }) => {
      const count = data.by_size[key];
      const pct = data.total > 0 ? Math.round((count / data.total) * 100) : 0;
      return { label, value: fmtNum(count), pct };
    });
    // Windowed total against its lifetime twin. Rendered ONLY when the
    // server sent the baseline — a substituted one (falling back to the
    // window itself) would read as a real comparison while comparing the
    // number to itself, which is worse than no comparison at all.
    const lifetime = data.lifetime;
    const compare = lifetime
      ? `${fmtNum(data.total)} of ${fmtNum(lifetime.total_stows)} stows all time`
      : null;
    // Trend WINS over the lifetime share when both exist: direction is
    // what a glance is for. Showing both put three clauses in the note,
    // which wrapped the tile and left dead space -- the sizing contract
    // is that a tile never scrolls and never wastes room. Lifetime stays
    // as the fallback so a handle with no predecessor still gets a
    // comparison rather than a bare number.
    const trend = data.previous
      ? formatTrend(
          computeTrend(data.total, data.previous.total_stows),
          previousWindowLabel(ctx.range),
          fmtNum,
        )
      : null;
    // The stowing caveat stays: it says where the numbers come from, which
    // neither comparison replaces.
    const note = [trend ?? compare, STOW_CAVEAT].filter(Boolean).join(' · ');
    return <MeterList header={header} meters={meters} note={note} />;
  },
});
