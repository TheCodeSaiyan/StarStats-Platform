import React from 'react';
import { prettyShop } from '@/lib/shop-name';
import { getCommerceRecent, getFriendCommerceRecent, getSpend } from '@/lib/api';
import type {
  CommerceRecentResponse,
  CommerceTransaction,
  SpendResponse,
} from '@/lib/api';
import { logger } from '@/lib/logger';
import { rangeToHours, rangeHasLifetimeBaseline } from '@/lib/range';
import { computeTrend, formatTrend, previousWindowLabel } from '@/lib/trend';
import { EmptyWindow } from './kit/EmptyWindow';
import { defineWidget } from './kit/defineWidget';
import { ReadoutGroup, RankedList, type Readout, type Row } from './kit/archetypes';
import { fmtNum } from './kit/format';

// Data source per viewer:
//   - Owner self-view:   /v1/me/commerce/recent       (existing endpoint)
//   - Friend visitor:    /v1/u/:handle/commerce/recent (Plan 3b A.2)
//   - Anonymous visitor: not available (no bearer to scope to)
//
// The friend path is gated server-side by
// `widget_allowed_for_scope(scope, "economy")` against the owner's
// share_metadata. A 404 from the friend endpoint = "not shared" or
// "widget denied" — load converts it to `null` for an empty card.

function isBuyKind(kind: CommerceTransaction['kind']): boolean {
  return kind === 'shop' || kind === 'commodity_buy';
}

interface EconomyData {
  buys: number;
  sells: number;
  confirmed: number;
  pending: number;
  perKind: Record<CommerceTransaction['kind'], number>;
  spend: SpendResponse | null;
}

export const economyWidget = defineWidget<EconomyData>({
  id: 'economy',
  eyebrow: 'Economy',
  rangeAware: true,
  // Owner always sees their own widget; a visitor needs the owner's
  // per-widget `economy` share toggle (Plan 3b Option A). The server also
  // enforces this at the friend endpoint.
  visibility: { shareScope: 'economy' },
  async load(ctx) {
    if (!ctx.token) return null;
    const token = ctx.token;
    const hours = rangeToHours(ctx.range);
    // Commerce list (owner or friend-scoped) + the me-scoped spend total.
    // Spend is owner-only, so visitors resolve it to null (no spend section).
    // Spend takes the SAME `hours` window as the commerce list: fetching it
    // unscoped put a lifetime aUEC total next to a range-scoped buy/sell
    // count under a single range label.
    const [commerceRes, spendRes] = await Promise.allSettled([
      ctx.isOwner
        ? getCommerceRecent(token, 100, 30, hours)
        : getFriendCommerceRecent(token, ctx.ownerHandle, 100, 30, hours),
      ctx.isOwner ? getSpend(token, hours) : Promise.resolve(null),
    ]);
    if (commerceRes.status === 'rejected') {
      logger.warn({ err: commerceRes.reason, call: 'widget.economy' }, 'fetch failed');
      return null;
    }
    if (spendRes.status === 'rejected') {
      logger.warn({ err: spendRes.reason, call: 'widget.economy.spend' }, 'fetch failed');
    }
    const resp: CommerceRecentResponse | null = commerceRes.value;
    const spend: SpendResponse | null =
      spendRes.status === 'fulfilled' ? spendRes.value : null;
    const txs = resp?.transactions ?? [];
    // An empty WINDOW is not an empty account — see kit/EmptyWindow.
    // Bailing here on `txs.length === 0` threw away a perfectly good
    // `spend` payload, so a handle whose trading predates the range got
    // the same blank box a brand-new account gets. Keep rendering
    // whenever there is a lifetime figure to name; a visitor (spend is
    // owner-only) or the `all` range still resolve to 0 and fall through
    // to `null`, because telling someone to widen a range that holds
    // nothing wider is worse than silence.
    const lifetimePurchases = rangeHasLifetimeBaseline(ctx.range)
      ? (spend?.lifetime?.purchases ?? 0)
      : 0;
    if (txs.length === 0 && lifetimePurchases === 0) return null;

    let buys = 0;
    let sells = 0;
    let confirmed = 0;
    let pending = 0;
    const perKind: Record<CommerceTransaction['kind'], number> = {
      shop: 0,
      commodity_buy: 0,
      commodity_sell: 0,
    };
    for (const tx of txs) {
      perKind[tx.kind] += 1;
      if (isBuyKind(tx.kind)) buys += 1;
      else sells += 1;
      if (tx.status === 'confirmed') confirmed += 1;
      else if (tx.status === 'pending' || tx.status === 'submitted') pending += 1;
    }

    return { buys, sells, confirmed, pending, perKind, spend };
  },
  body(data, ctx, size) {
    const { buys, sells, confirmed, pending, perKind, spend } = data;
    // Nothing traded in this window, but the account has traded before.
    // `load` only lets us reach here with a lifetime figure to name.
    // Counted in purchases, not aUEC: EmptyWindow renders the number
    // beside "all time", and a spend total sitting next to the word
    // "purchases" would read as a count of something it isn't.
    if (buys === 0 && sells === 0) {
      return (
        <EmptyWindow
          rangeLabel={previousWindowLabel(ctx.range)}
          lifetimeCount={spend?.lifetime?.purchases ?? 0}
          noun="purchases"
        />
      );
    }
    // NO aUEC FIGURE HERE. `spend` is the money widget and carries a better
    // version of every part of it: the same total with an InfoTip explaining
    // the kiosk inference, a lifetime baseline covering both its readouts,
    // and a trend. This tile was rendering a poorer copy TWICE — a readout
    // and a `Spent` row — off the same getSpend call, two tiles away in a
    // two-tile lens. economy owns transaction activity; spend owns money.
    //
    // The getSpend call itself STAYS: `lifetimePurchases` above uses it to
    // tell an empty WINDOW from an empty ACCOUNT, which is the difference
    // between "widen the range" and a blank box.

    if (size === 'compact') {
      const readouts: Readout[] = [
        { label: 'buys', value: fmtNum(buys) },
        { label: 'sells', value: fmtNum(sells) },
      ];
      return (
        <ReadoutGroup
          readouts={readouts}
          note={
            <>
              {fmtNum(confirmed)} confirmed
              {pending > 0 && <> · {fmtNum(pending)} pending</>}
            </>
          }
        />
      );
    }

    const rows: Row[] = [
      { key: 'shop', label: 'Shop', value: fmtNum(perKind.shop) },
      { key: 'commodity_buy', label: 'Commodity buy', value: fmtNum(perKind.commodity_buy) },
      { key: 'commodity_sell', label: 'Commodity sell', value: fmtNum(perKind.commodity_sell) },
    ];
    return (
      <RankedList
        rows={rows}
        note={
          <>
            {fmtNum(confirmed)} confirmed · {fmtNum(pending)} pending
            {spend?.top_shop && <> · top shop {prettyShop(spend.top_shop)}</>}
          </>
        }
      />
    );
  },
});
