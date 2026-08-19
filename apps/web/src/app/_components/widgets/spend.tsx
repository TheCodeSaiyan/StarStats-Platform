import React from 'react';
import { getSpend } from '@/lib/api';
import { logger } from '@/lib/logger';
import { rangeToWindowHours, rangeHasLifetimeBaseline } from '@/lib/range';
import { computeTrend, formatTrend, previousWindowLabel } from '@/lib/trend';
import { EmptyWindow } from './kit/EmptyWindow';
import { defineWidget } from './kit/defineWidget';
import { ReadoutGroup, type Readout } from './kit/archetypes';
import { fmtNum } from './kit/format';
import { InfoTip } from '@/components/hud/InfoTip';
import { INFERENCE_EXPLANATIONS } from '@/lib/inference-explanations';

/** Strip the `SCShop_` prefix + underscores from a raw shop_name. */
function prettyShop(raw: string): string {
  return raw.replace(/^SCShop[_-]?/i, '').replace(/_/g, ' ').trim() || raw;
}

/**
 * `spend` widget — kiosk spending depth from the reparse-gated
 * `GET /v1/me/stats/spend` aggregate (`shop_buy_request.price`): total
 * aUEC spent, purchase count, and the top shop. Owner-only (me-scoped),
 * range-aware (follows the dashboard range selector).
 *
 * Complements the `economy` widget (which counts buys/sells in the same
 * window); this is the spend-depth headline over the selected range. Min-
 * viable datum: total aUEC. The purchases + top-shop line is secondary and
 * drops at min width.
 */
interface SpendData {
  total_auec: number;
  purchases: number;
  top_shop: string | null;
  /** Lifetime baseline the windowed figures above are read against — "15,000
   *  aUEC" says nothing on its own about whether that is a lot. Null on the
   *  `all` range — it spans the full 365 days of retention, so its twin
   *  covers the same rows and a comparison would only restate the figures —
   *  and null whenever the server sent no twin. Never substituted or
   *  estimated: a fabricated baseline is worse than a bare number. */
  lifetime: { total_auec: number; purchases: number } | null;
  /** Same-length window immediately before this one. Null means "no
   *  comparison to draw" — the server omits it on `all` (no predecessor
   *  inside retention) and for a handle with no prior activity at all.
   *  NEVER coerce to 0: a real zero means "played, spent nothing", which
   *  is a genuine comparison; absent means we have none to make. */
  previous: { total_auec: number; purchases: number } | null;
}

export const spendWidget = defineWidget<SpendData>({
  id: 'spend',
  eyebrow: 'Spending',
  rangeAware: true,
  // Owner-only: /v1/me/stats/spend has no friend-scoped equivalent.
  visibility: 'owner',
  async load(ctx) {
    if (!ctx.token) return null;
    let spend = null;
    try {
      spend = await getSpend(ctx.token, rangeToWindowHours(ctx.range));
    } catch (err) {
      logger.warn({ err, call: 'widget.spend' }, 'fetch failed');
      return null;
    }
    // An empty WINDOW is not an empty account — see kit/EmptyWindow.
    const lifetimeSpent = rangeHasLifetimeBaseline(ctx.range)
      ? (spend?.lifetime?.total_auec ?? 0)
      : 0;
    if (
      !spend ||
      (spend.total_auec === 0 && spend.purchases === 0 && lifetimeSpent === 0)
    ) {
      return null;
    }
    return {
      total_auec: spend.total_auec,
      purchases: spend.purchases,
      top_shop: spend.top_shop ?? null,
      // Dropped on `all`: that range already spans retention, so the
      // twin covers the same rows and the note would read "N of N".
      lifetime:
        spend.lifetime && rangeHasLifetimeBaseline(ctx.range)
          ? {
              total_auec: spend.lifetime.total_auec,
              purchases: spend.lifetime.purchases,
            }
          : null,
      // Not range-gated here: the server already omits `previous` for
      // `all` and for a handle with no prior activity. Re-deciding it
      // client-side would risk the two rules drifting apart.
      previous: spend.previous
        ? {
            total_auec: spend.previous.total_auec,
            purchases: spend.previous.purchases,
          }
        : null,
    };
  },
  body(data, ctx) {
    if (data.total_auec === 0 && data.purchases === 0) {
      return (
        <EmptyWindow
          rangeLabel={previousWindowLabel(ctx.range)}
          // Purchases, not aUEC: EmptyWindow renders this beside "all
          // time" under the noun, so passing the spend total read as
          // "1,250,000 purchases all time" — a currency figure wearing
          // a count's label.
          lifetimeCount={data.lifetime?.purchases ?? 0}
          noun="purchases"
        />
      );
    }
    const readouts: Readout[] = [
      {
        label: 'spent',
        info: <InfoTip label="spend" text={INFERENCE_EXPLANATIONS.kiosk_spend} />,
        value: `${fmtNum(data.total_auec)} aUEC`,
      },
      { label: 'purchases', value: fmtNum(data.purchases) },
    ];
    // One baseline covering BOTH readouts, so the window reads as a share of
    // a career rather than two bare magnitudes. Dropped entirely (not
    // zero-filled) when the twin is absent — see `SpendData.lifetime`.
    const lt = data.lifetime;
    const lifetimeNote = lt
      ? `Lifetime — ${fmtNum(lt.total_auec)} aUEC, ${fmtNum(lt.purchases)} purchases`
      : undefined;
    // Trend leads the note: direction answers a question the lifetime
    // share cannot. Measured on SPEND, not purchases — spend is the
    // headline figure and two trends in one line reads as noise.
    const trend = data.previous
      ? formatTrend(
          computeTrend(data.total_auec, data.previous.total_auec),
          previousWindowLabel(ctx.range),
          fmtNum,
          'aUEC',
        )
      : undefined;
    const note = trend ?? lifetimeNote;
    return (
      <div>
        <ReadoutGroup readouts={readouts} note={note} />
        {data.top_shop ? (
          <p className="hud-note hud-secondary">Top shop: {prettyShop(data.top_shop)}</p>
        ) : null}
      </div>
    );
  },
});
