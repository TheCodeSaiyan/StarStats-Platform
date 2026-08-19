import React from 'react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render } from '@testing-library/react';

vi.mock('@/lib/api', () => ({
  getSpend: vi.fn(),
}));

import { getSpend } from '@/lib/api';
import { spendWidget } from './spend';
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

const mockSpend = () => getSpend as ReturnType<typeof vi.fn>;

describe('spendWidget lifetime comparison', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('compares the window against the lifetime baseline when the API sends one', async () => {
    mockSpend().mockResolvedValue({
      total_auec: 17_500,
      purchases: 3,
      top_shop: 'SCShop_Aparelli_NewBabbage',
      lifetime: { total_auec: 1_250_000, purchases: 412 },
    });

    const { container } = render(<>{await spendWidget.render(ownerCtx('7d'), 'expanded')}</>);

    // The windowed headline is unchanged...
    expect(container.textContent).toContain('17,500 aUEC');
    // ...and now carries the baseline that makes it mean something.
    expect(container.textContent).toContain('Lifetime — 1,250,000 aUEC, 412 purchases');
  });

  it('renders the bare number with NO comparison when lifetime is absent', async () => {
    // No window was requested (the 'all' range sends no `hours`), so the
    // server omits the twin — the figures ARE lifetime. Anything rendered
    // here would be an invented baseline, which is worse than no baseline.
    mockSpend().mockResolvedValue({
      total_auec: 17_500,
      purchases: 3,
      top_shop: 'SCShop_Aparelli_NewBabbage',
    });

    const { container } = render(<>{await spendWidget.render(ownerCtx('all'), 'expanded')}</>);

    expect(container.textContent).toContain('17,500 aUEC');
    expect(container.textContent).not.toContain('Lifetime');
    // Guards the specific failure mode: a `?? 0` fallback rendering a
    // fabricated "0 aUEC" career total next to the real window.
    expect(container.textContent).not.toMatch(/0 aUEC, 0 purchases/);
  });

  it('treats an explicit null lifetime as absent', async () => {
    // The wire type is `null | SpendLifetime`; a plain `!== undefined`
    // check would let a null through into the note.
    mockSpend().mockResolvedValue({
      total_auec: 17_500,
      purchases: 3,
      top_shop: null,
      lifetime: null,
    });

    const { container } = render(<>{await spendWidget.render(ownerCtx('all'), 'expanded')}</>);

    expect(container.textContent).toContain('17,500 aUEC');
    expect(container.textContent).not.toContain('Lifetime');
  });

  it('keeps the top-shop line, which is a component and not a baseline', async () => {
    mockSpend().mockResolvedValue({
      total_auec: 17_500,
      purchases: 3,
      top_shop: 'SCShop_Aparelli_NewBabbage',
      lifetime: { total_auec: 1_250_000, purchases: 412 },
    });

    const { container } = render(<>{await spendWidget.render(ownerCtx('7d'), 'expanded')}</>);

    expect(container.textContent).toContain('Top shop: Aparelli NewBabbage');
  });

  // The server DOES send a twin for `all` (it is a real 8760h window),
  // but on that range the twin spans the same rows as the window, so
  // the comparison would read "1,250,000 of 1,250,000". Suppressed
  // client-side: a comparison that restates its own figure occupies
  // the space where real context belongs.
  it('renders no comparison on the "all" range even when the API sends a twin', async () => {
    mockSpend().mockResolvedValue({
      total_auec: 1_250_000,
      purchases: 412,
      top_shop: null,
      lifetime: { total_auec: 1_250_000, purchases: 412 },
    });

    const { container } = render(
      <>{await spendWidget.render(ownerCtx('all'), 'expanded')}</>,
    );

    expect(container.textContent).not.toMatch(/lifetime/i);
  });

  // `all` is 365 days, not lifetime: 365 days IS the hard retention
  // limit, so "everything we have" and "the last year" are the same
  // set. Sending `undefined` here would promise a depth the data does
  // not have, and would diverge from the server, which bounds `all`
  // to 365 as well.
  it('asks for 365 days when the range is "all"', async () => {
    mockSpend().mockResolvedValue({ total_auec: 1, purchases: 1, top_shop: null });

    await spendWidget.render(ownerCtx('all'), 'expanded');

    expect(getSpend).toHaveBeenCalledWith('tok', 8760);
  });

  // Without `previous` seeded the widget takes the `?? lifetime`
  // fallback, so the trend branch is unreachable and a wrong field or
  // inverted direction stays green.
  it('leads with the trend and drops the lifetime line when a predecessor exists', async () => {
    mockSpend().mockResolvedValue({
      total_auec: 17_500,
      purchases: 3,
      top_shop: null,
      lifetime: { total_auec: 1_250_000, purchases: 412 },
      previous: { total_auec: 14_000, purchases: 2 },
    });

    const { container } = render(<>{await spendWidget.render(ownerCtx('7d'), 'expanded')}</>);

    expect(container.textContent).toContain('+3,500 aUEC');
    expect(container.textContent).toContain('(+25%)');
    expect(container.textContent).toContain('vs prev 7d');
    // Trend REPLACES the lifetime line rather than joining it.
    expect(container.textContent).not.toContain('Lifetime —');
  });

  it('reports a fall in spend as a fall', async () => {
    mockSpend().mockResolvedValue({
      total_auec: 5_000,
      purchases: 1,
      top_shop: null,
      previous: { total_auec: 20_000, purchases: 6 },
    });
    const { container } = render(<>{await spendWidget.render(ownerCtx('7d'), 'expanded')}</>);
    expect(container.textContent).toContain('−15,000 aUEC');
    expect(container.textContent).toContain('(−75%)');
  });
});

// #363 gave these tiles an empty-window state; nothing pinned that it
// actually renders. See kit/EmptyWindow for the bug it fixes.
describe('spendWidget empty window vs empty account', () => {
  beforeEach(() => { vi.clearAllMocks(); });

  it('says the window is empty rather than going blank when lifetime has purchases', async () => {
    mockSpend().mockResolvedValue({
      total_auec: 0,
      purchases: 0,
      top_shop: null,
      lifetime: { total_auec: 1_250_000, purchases: 412 },
    });
    const node = await spendWidget.render(ownerCtx('7d'), 'compact');
    expect(node).not.toBeNull();
    const { container } = render(<>{node}</>);
    expect(container.textContent).toMatch(/7d/);
    expect(container.textContent).toMatch(/widen the range/i);
    // The count must be PURCHASES, not the aUEC total: the noun is
    // "purchases", so rendering 1,250,000 there labels a currency
    // figure as a count of trades.
    expect(container.textContent).toContain('412');
    expect(container.textContent).not.toContain('1,250,000');
  });

  it('renders nothing at all when there are no purchases in any window', async () => {
    mockSpend().mockResolvedValue({
      total_auec: 0, purchases: 0, top_shop: null,
      lifetime: { total_auec: 0, purchases: 0 },
    });
    expect(await spendWidget.render(ownerCtx('7d'), 'compact')).toBeNull();
  });

  it('renders nothing on the "all" range when the window is empty', async () => {
    mockSpend().mockResolvedValue({
      total_auec: 0, purchases: 0, top_shop: null,
      lifetime: { total_auec: 1_250_000, purchases: 412 },
    });
    expect(await spendWidget.render(ownerCtx('all'), 'compact')).toBeNull();
  });
});
