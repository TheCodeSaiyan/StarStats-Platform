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

describe('economyWidget lifetime suppression on "all"', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  // `all` is a real 8760h window, so the server DOES send a twin for it
  // — but that twin spans the same rows, so the note would read
  // "lifetime 1,250,000 aUEC" beside an identical windowed figure.
  it('renders no lifetime note on the "all" range', async () => {
    (getCommerceRecent as ReturnType<typeof vi.fn>).mockResolvedValue({
      transactions: [
        { kind: 'shop', status: 'confirmed', shop_name: 'SCShop_X' },
      ],
    });
    (getSpend as ReturnType<typeof vi.fn>).mockResolvedValue({
      total_auec: 1_250_000,
      purchases: 412,
      top_shop: null,
      lifetime: { total_auec: 1_250_000, purchases: 412 },
    });

    const node = await economyWidget.render(ownerCtx('all'), 'expanded');
    const { container } = render(node as React.ReactElement);

    expect(container.textContent).toContain('1,250,000 aUEC');
    expect(container.textContent).not.toMatch(/lifetime/i);
  });
});

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

  it('renders the spend total + top shop from getSpend', async () => {
    (getCommerceRecent as ReturnType<typeof vi.fn>).mockResolvedValue({
      transactions: [{ kind: 'shop', status: 'confirmed' }],
    });
    (getSpend as ReturnType<typeof vi.fn>).mockResolvedValue({
      total_auec: 17500,
      purchases: 3,
      top_shop: 'SCShop_Aparelli_NewBabbage',
    });

    const node = await economyWidget.render(ownerCtx('7d'), 'expanded');
    const { container } = render(node as React.ReactElement);

    // 7d => 168 hours. Spend MUST share the commerce window, not be lifetime.
    expect(getSpend).toHaveBeenCalledWith('tok', 168);
    expect(container.textContent).toContain('17,500 aUEC');
    // shop_name is prettified (SCShop_ prefix + underscores stripped).
    expect(container.textContent).toContain('Aparelli NewBabbage');
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

  it('compares the windowed spend against the lifetime baseline when present', async () => {
    (getCommerceRecent as ReturnType<typeof vi.fn>).mockResolvedValue({
      transactions: [{ kind: 'shop', status: 'confirmed' }],
    });
    (getSpend as ReturnType<typeof vi.fn>).mockResolvedValue({
      total_auec: 17_500,
      purchases: 3,
      top_shop: 'SCShop_Aparelli_NewBabbage',
      lifetime: { total_auec: 1_250_000, purchases: 412 },
    });

    const node = await economyWidget.render(ownerCtx('7d'), 'expanded');
    const { container } = render(node as React.ReactElement);

    expect(container.textContent).toContain('17,500 aUEC');
    expect(container.textContent).toContain('lifetime 1,250,000 aUEC');
    // The pre-existing confirmed/pending breakdown is a separate, still-true
    // caveat about the transaction counts — the comparison JOINS it.
    expect(container.textContent).toContain('1 confirmed');
    expect(container.textContent).toContain('top shop Aparelli NewBabbage');
  });

  it('renders the bare spend with NO comparison when lifetime is absent', async () => {
    (getCommerceRecent as ReturnType<typeof vi.fn>).mockResolvedValue({
      transactions: [{ kind: 'shop', status: 'confirmed' }],
    });
    (getSpend as ReturnType<typeof vi.fn>).mockResolvedValue({
      total_auec: 17_500,
      purchases: 3,
      top_shop: null,
    });

    const node = await economyWidget.render(ownerCtx('7d'), 'expanded');
    const { container } = render(node as React.ReactElement);

    expect(container.textContent).toContain('17,500 aUEC');
    expect(container.textContent).not.toContain('lifetime');
    // Guards the `?? 0` failure mode: a fabricated "0 aUEC" career total.
    expect(container.textContent).not.toMatch(/lifetime 0 aUEC/);
    // Existing note survives.
    expect(container.textContent).toContain('1 confirmed');
  });

  it('carries the lifetime comparison into the compact size too', async () => {
    (getCommerceRecent as ReturnType<typeof vi.fn>).mockResolvedValue({
      transactions: [{ kind: 'shop', status: 'confirmed' }],
    });
    (getSpend as ReturnType<typeof vi.fn>).mockResolvedValue({
      total_auec: 17_500,
      purchases: 3,
      top_shop: null,
      lifetime: { total_auec: 1_250_000, purchases: 412 },
    });

    const node = await economyWidget.render(ownerCtx('7d'), 'compact');
    const { container } = render(node as React.ReactElement);

    // `spent` is displayed at compact too, so it needs the baseline there.
    expect(container.textContent).toContain('17,500 aUEC');
    expect(container.textContent).toContain('lifetime 1,250,000 aUEC');
  });

  it('omits the comparison when there is no spend figure to compare', async () => {
    // A baseline hanging off nothing is noise: with no windowed spend the
    // widget shows no aUEC at all.
    (getCommerceRecent as ReturnType<typeof vi.fn>).mockResolvedValue({
      transactions: [{ kind: 'shop', status: 'confirmed' }],
    });
    (getSpend as ReturnType<typeof vi.fn>).mockResolvedValue({
      total_auec: 0,
      purchases: 0,
      top_shop: null,
      lifetime: { total_auec: 1_250_000, purchases: 412 },
    });

    const node = await economyWidget.render(ownerCtx('7d'), 'expanded');
    const { container } = render(node as React.ReactElement);

    expect(container.textContent).not.toContain('aUEC');
    expect(container.textContent).not.toContain('lifetime');
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

  // The trend branch is unreachable without `previous`: the widget
  // renders `spendTrend ?? lifetimeSpend`, so every other assertion in
  // this file exercises only the fallback.
  it('leads with the spend trend and drops the lifetime note', async () => {
    (getCommerceRecent as ReturnType<typeof vi.fn>).mockResolvedValue({
      transactions: [
        { kind: 'shop', status: 'confirmed', shop_name: 'SCShop_X' },
      ],
    });
    (getSpend as ReturnType<typeof vi.fn>).mockResolvedValue({
      total_auec: 30_000,
      purchases: 5,
      top_shop: null,
      lifetime: { total_auec: 1_250_000, purchases: 412 },
      previous: { total_auec: 20_000, purchases: 4 },
    });

    const node = await economyWidget.render(ownerCtx('7d'), 'expanded');
    const { container } = render(node as React.ReactElement);

    expect(container.textContent).toContain('+10,000 aUEC');
    expect(container.textContent).toContain('(+50%)');
    expect(container.textContent).toContain('vs prev 7d');
    // Trend REPLACES the lifetime note rather than joining it.
    expect(container.textContent).not.toContain('lifetime 1,250,000');
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
