import React from 'react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render } from '@testing-library/react';

vi.mock('@/lib/api', () => ({
  getCommerceRecent: vi.fn(),
  getFriendCommerceRecent: vi.fn(),
  getSpend: vi.fn(),
}));

import { getCommerceRecent, getSpend } from '@/lib/api';
import { economyWidget } from './economy';
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

describe('economyWidget range-awareness', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('is marked range-aware', () => {
    expect(economyWidget.rangeAware).toBe(true);
  });

  it('passes the ctx.range window (hours) to getCommerceRecent', async () => {
    (getCommerceRecent as ReturnType<typeof vi.fn>).mockResolvedValue({
      transactions: [{ kind: 'shop', status: 'confirmed' }],
    });

    await economyWidget.render(ownerCtx('7d'), 'compact');

    // 7d => 24*7 = 168 hours, passed as the 4th arg.
    expect(getCommerceRecent).toHaveBeenCalledWith('tok', 100, 30, 168);
  });

  it('passes the ctx.range window (hours) to getSpend', async () => {
    (getCommerceRecent as ReturnType<typeof vi.fn>).mockResolvedValue({
      transactions: [{ kind: 'shop', status: 'confirmed' }],
    });
    (getSpend as ReturnType<typeof vi.fn>).mockResolvedValue({
      total_auec: 500,
      purchases: 1,
      top_shop: null,
    });

    await economyWidget.render(ownerCtx('30d'), 'compact');

    // 30d => 24*30 = 720 hours, passed as the 2nd arg.
    expect(getSpend).toHaveBeenCalledWith('tok', 720);
  });

  it('scopes spend to the SAME window as the commerce list', async () => {
    // Regression guard: `getSpend(token)` with no hours returned a
    // lifetime aUEC total that rendered next to a range-scoped buy/sell
    // count under one range label.
    (getCommerceRecent as ReturnType<typeof vi.fn>).mockResolvedValue({
      transactions: [{ kind: 'shop', status: 'confirmed' }],
    });
    (getSpend as ReturnType<typeof vi.fn>).mockResolvedValue({
      total_auec: 500,
      purchases: 1,
      top_shop: null,
    });

    await economyWidget.render(ownerCtx('90d'), 'compact');

    const commerceHours = (getCommerceRecent as ReturnType<typeof vi.fn>).mock.calls[0][3];
    const spendArgs = (getSpend as ReturnType<typeof vi.fn>).mock.calls[0];
    expect(spendArgs).toHaveLength(2);
    expect(spendArgs[1]).toBe(commerceHours);
    expect(spendArgs[1]).toBe(2160);
  });
});

// Economy was the one range-aware tile #363 did not give an empty-window
// state, so an empty commerce list bailed out of `load` entirely — and
// took a perfectly good `spend` payload with it. Same reported symptom
// as routes: a blank tile that reads as a broken feature.
describe('economyWidget empty window vs empty account', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('says the window is empty rather than going blank when lifetime has purchases', async () => {
    (getCommerceRecent as ReturnType<typeof vi.fn>).mockResolvedValue({
      transactions: [],
    });
    (getSpend as ReturnType<typeof vi.fn>).mockResolvedValue({
      total_auec: 0,
      purchases: 0,
      top_shop: null,
      lifetime: { total_auec: 1_250_000, purchases: 412 },
    });

    const node = await economyWidget.render(ownerCtx('7d'), 'compact');
    expect(node).not.toBeNull();
    const { container } = render(node as React.ReactElement);
    // Names the lifetime figure, the window, and the fix.
    expect(container.textContent).toContain('412');
    expect(container.textContent).toMatch(/7d/);
    expect(container.textContent).toMatch(/widen the range/i);
    // Counted in PURCHASES, not aUEC — "1,250,000 all time" beside the
    // word "purchases" would name a spend total as a count of trades.
    expect(container.textContent).not.toContain('1,250,000');
  });

  it('renders nothing at all when there are no purchases in any window', async () => {
    (getCommerceRecent as ReturnType<typeof vi.fn>).mockResolvedValue({
      transactions: [],
    });
    (getSpend as ReturnType<typeof vi.fn>).mockResolvedValue({
      total_auec: 0,
      purchases: 0,
      top_shop: null,
      lifetime: { total_auec: 0, purchases: 0 },
    });

    expect(await economyWidget.render(ownerCtx('7d'), 'compact')).toBeNull();
  });

  it('renders nothing on the "all" range when the list is empty', async () => {
    // `all` spans retention — there is no wider range to widen to, so
    // suggesting one would send the user nowhere.
    (getCommerceRecent as ReturnType<typeof vi.fn>).mockResolvedValue({
      transactions: [],
    });
    (getSpend as ReturnType<typeof vi.fn>).mockResolvedValue({
      total_auec: 0,
      purchases: 0,
      top_shop: null,
      lifetime: { total_auec: 1_250_000, purchases: 412 },
    });

    expect(await economyWidget.render(ownerCtx('all'), 'compact')).toBeNull();
  });

  it('renders nothing when there is no spend payload to name a lifetime from', async () => {
    // Visitors get no spend (owner-only endpoint), so there is no
    // lifetime figure — and EmptyWindow must not invent one.
    (getCommerceRecent as ReturnType<typeof vi.fn>).mockResolvedValue({
      transactions: [],
    });
    (getSpend as ReturnType<typeof vi.fn>).mockResolvedValue(null);

    expect(await economyWidget.render(ownerCtx('7d'), 'compact')).toBeNull();
  });
});

// The spend-display tests that stood here are gone with the display itself.
// Every one of their subjects is already covered in `spend.test.tsx` — the
// lifetime baseline, the bare-number case, the `all`-range rule, the
// trend-over-lifetime precedence and the top-shop line. The duplication this
// change removes ran to the TESTS as well as the code: two widgets, two
// suites, the same assertions about the same getSpend call.

describe('economy does not restate the spend widget', () => {
  beforeEach(() => vi.clearAllMocks());

  async function renderWithSpend(size: 'compact' | 'expanded') {
    (getCommerceRecent as ReturnType<typeof vi.fn>).mockResolvedValue({
      transactions: [
        { kind: 'shop', status: 'confirmed', shop_name: 'SCShop_X' },
        { kind: 'commodity_buy', status: 'confirmed', shop_name: null },
      ],
    });
    (getSpend as ReturnType<typeof vi.fn>).mockResolvedValue({
      total_auec: 1_250_000,
      purchases: 412,
      top_shop: null,
      lifetime: { total_auec: 4_000_000, purchases: 900 },
      previous: { total_auec: 1_000_000, purchases: 300 },
    });
    const node = await economyWidget.render(ownerCtx('7d'), size);
    return render(node as React.ReactElement).container;
  }

  it('shows no aUEC figure in either size', async () => {
    for (const size of ['compact', 'expanded'] as const) {
      const c = await renderWithSpend(size);
      expect(c.textContent, `${size} must not restate spend`).not.toContain('aUEC');
    }
  });

  it('keeps the activity figures it alone can report', async () => {
    const c = await renderWithSpend('compact');
    expect(c.textContent).toContain('buys');
    expect(c.textContent).toContain('sells');
    expect(c.textContent).toContain('confirmed');
  });

  it('still renders when the window is empty but the account is not', async () => {
    // The getSpend call STAYS even though its figure does not: the lifetime
    // purchase count is what tells an empty WINDOW from an empty ACCOUNT.
    (getCommerceRecent as ReturnType<typeof vi.fn>).mockResolvedValue({
      transactions: [],
    });
    (getSpend as ReturnType<typeof vi.fn>).mockResolvedValue({
      total_auec: 0,
      purchases: 0,
      top_shop: null,
      lifetime: { total_auec: 4_000_000, purchases: 900 },
    });
    const node = await economyWidget.render(ownerCtx('7d'), 'compact');
    expect(node).not.toBeNull();
  });
});

// The counts on this tile used to be `transactions.length`, which is the
// PAGE size the widget asked for. Everyone who traded more than a hundred
// times in the window saw "Buys 100" — the cap, not their data. The server
// now sends `totals`, counted over the whole window.
describe('economyWidget counts come from totals, not the page', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  function pageOf(n: number) {
    return Array.from({ length: n }, () => ({ kind: 'shop', status: 'confirmed' }));
  }

  // `WidgetDef.load` is optional on the registry type (types.ts:139) because
  // not every widget fetches. These assertions are about what the loader
  // computes, so pin it once rather than repeating a non-null assertion.
  async function loadOwner(range: ViewerCtx['range']) {
    const load = economyWidget.load;
    if (!load) throw new Error('economyWidget must define a loader');
    return (await load(ownerCtx(range))) as {
      buys: number;
      sells: number;
      sampled: boolean;
      shown: number;
    };
  }

  it('reports the windows true buy count when the page is capped', async () => {
    (getCommerceRecent as ReturnType<typeof vi.fn>).mockResolvedValue({
      transactions: pageOf(100),
      totals: { shop: 3412, commodity_buy: 0, commodity_sell: 0 },
    });
    (getSpend as ReturnType<typeof vi.fn>).mockResolvedValue({
      total_auec: 9_100_000,
      purchases: 3412,
      top_shop: null,
    });

    const data = await loadOwner('30d');

    expect(data.buys).toBe(3412);
    expect(data.buys).not.toBe(100);
    expect(data.sampled).toBe(true);
    expect(data.shown).toBe(100);
  });

  it('adds commodity buys into buys and keeps sells separate', async () => {
    (getCommerceRecent as ReturnType<typeof vi.fn>).mockResolvedValue({
      transactions: pageOf(3),
      totals: { shop: 200, commodity_buy: 45, commodity_sell: 17 },
    });
    (getSpend as ReturnType<typeof vi.fn>).mockResolvedValue(null);

    const data = await loadOwner('30d');

    expect(data.buys).toBe(245);
    expect(data.sells).toBe(17);
  });

  it('says the status line describes the sample when the page is capped', async () => {
    (getCommerceRecent as ReturnType<typeof vi.fn>).mockResolvedValue({
      transactions: pageOf(100),
      totals: { shop: 3412, commodity_buy: 0, commodity_sell: 0 },
    });
    (getSpend as ReturnType<typeof vi.fn>).mockResolvedValue(null);

    const node = await economyWidget.render(ownerCtx('30d'), 'compact');
    const { container } = render(node as React.ReactElement);

    expect(container.textContent).toContain('3,412');
    // "100 confirmed" beside "3,412 buys" would imply 3,312 unanswered.
    expect(container.textContent).toMatch(/newest 100/i);
  });

  it('falls back to counting the page when the API predates totals', async () => {
    // Rollout window only: web and API containers roll independently.
    (getCommerceRecent as ReturnType<typeof vi.fn>).mockResolvedValue({
      transactions: pageOf(7),
    });
    (getSpend as ReturnType<typeof vi.fn>).mockResolvedValue(null);

    const data = await loadOwner('30d');

    expect(data.buys).toBe(7);
    expect(data.sampled).toBe(false);
  });
});
