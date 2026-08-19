import React from 'react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render } from '@testing-library/react';

vi.mock('@/lib/api', () => ({
  getDocking: vi.fn(),
}));

import { getDocking } from '@/lib/api';
import { dockingWidget } from './docking';
import { DEFAULT_SHARE_SCOPES, type ViewerCtx } from './types';

function ownerCtx(
  isOwner = true,
  range: ViewerCtx['range'] = '30d',
): ViewerCtx {
  return {
    ownerHandle: 'alice',
    viewerHandle: isOwner ? 'alice' : 'bob',
    isOwner,
    token: 'tok',
    shareScopes: { ...DEFAULT_SHARE_SCOPES },
    recipientScopes: null,
    range,
  };
}

function fixture() {
  return {
    total_stows: 10,
    by_kind: { hangar: 6, pad: 3, other: 1 },
    by_size: { small: 5, medium: 3, large: 2, xl: 0 },
  };
}

const mockDocking = () => getDocking as ReturnType<typeof vi.fn>;

describe('dockingWidget', () => {
  beforeEach(() => { vi.clearAllMocks(); });

  it('is range-aware and owner-only', async () => {
    expect(dockingWidget.rangeAware ?? false).toBe(true);
    expect(await dockingWidget.isAvailable(ownerCtx(true))).toBe(true);
    expect(await dockingWidget.isAvailable(ownerCtx(false))).toBe(false);
  });

  it('renders hangar/pad split, size meters and the stowing caveat', async () => {
    mockDocking().mockResolvedValue(fixture());
    const node = await dockingWidget.render(ownerCtx(), 'compact');
    const { container } = render(node as React.ReactElement);
    // hangar count + a size row (share now renders as the meter bar width,
    // not inline text) + the stowing caveat.
    expect(container.textContent).toContain('6');
    expect(container.textContent).toContain('Small');
    const fill = container.querySelector('.hud-meter__fill');
    expect(fill).not.toBeNull();
    // small = 5 / total 10 → 50% bar width.
    expect(fill?.getAttribute('style')).toContain('50%');
    expect(container.textContent).toContain('From ship stowing');
  });

  it('compares the windowed stows against the lifetime baseline', async () => {
    mockDocking().mockResolvedValue({ ...fixture(), lifetime: { total_stows: 412 } });
    const node = await dockingWidget.render(ownerCtx(), 'compact');
    const { container } = render(node as React.ReactElement);
    // Window total beside its lifetime twin — the baseline figure must be
    // on screen, not just implied.
    expect(container.textContent).toContain('10 of 412 stows all time');
    // The provenance caveat is joined to the comparison, not replaced by it.
    expect(container.textContent).toContain('From ship stowing');
  });

  // `all` is a real 8760h window, so the server DOES send a twin for it
  // — but that twin spans the same rows, so the note would read "412 of
  // 412 stows all time". Suppressed client-side; the bare caveat stays.
  it('renders no comparison on the "all" range even when the API sends a twin', async () => {
    mockDocking().mockResolvedValue({
      ...fixture(),
      total_stows: 412,
      lifetime: { total_stows: 412 },
    });
    const node = await dockingWidget.render(ownerCtx(true, 'all'), 'compact');
    const { container } = render(node as React.ReactElement);
    expect(container.textContent).not.toMatch(/all time/i);
    expect(container.textContent).toContain('From ship stowing');
  });

  // `lifetime` is omitted whenever no window was requested (the 'all'
  // range). Substituting a baseline there would compare the number to
  // itself while reading as a real comparison — worse than a bare number.
  it.each([
    ['absent', {}],
    ['null', { lifetime: null }],
  ])('renders no comparison when the lifetime baseline is %s', async (_label, extra) => {
    mockDocking().mockResolvedValue({ ...fixture(), ...extra });
    const node = await dockingWidget.render(ownerCtx(), 'compact');
    const { container } = render(node as React.ReactElement);
    // Own numbers and caveat still render …
    expect(container.textContent).toContain('6');
    expect(container.textContent).toContain('From ship stowing');
    // … but nothing that reads as a comparison against a baseline.
    expect(container.textContent).not.toContain('all time');
    expect(container.textContent).not.toMatch(/\d+ of \d+/);
  });

  it('returns null when there are no stows', async () => {
    mockDocking().mockResolvedValue({
      total_stows: 0,
      by_kind: { hangar: 0, pad: 0, other: 0 },
      by_size: { small: 0, medium: 0, large: 0, xl: 0 },
    });
    expect(await dockingWidget.render(ownerCtx(), 'compact')).toBeNull();
  });

  it('returns null when the fetch rejects', async () => {
    mockDocking().mockRejectedValue(new Error('boom'));
    expect(await dockingWidget.render(ownerCtx(), 'compact')).toBeNull();
  });

  // The trend branch was previously unreachable in tests: with no
  // `previous` in the fixture, `trend ?? compare` always took the
  // lifetime fallback, so a wrong field or an inverted direction would
  // have stayed green.
  it('leads with the period-over-period trend when a predecessor exists', async () => {
    mockDocking().mockResolvedValue({
      ...fixture(),
      lifetime: { total_stows: 412 },
      previous: { total_stows: 8 },
    });
    const node = await dockingWidget.render(ownerCtx(), 'compact');
    const { container } = render(node as React.ReactElement);
    // 10 this window vs 8 before: +2. Below the percentage floor, so no
    // percentage — 8 -> 10 is "+2", not a 25% swing worth reporting.
    expect(container.textContent).toContain('+2 vs prev 30d');
    expect(container.textContent).not.toContain('%');
    // Trend REPLACES the lifetime line rather than joining it.
    expect(container.textContent).not.toContain('all time');
    expect(container.textContent).toContain('From ship stowing');
  });

  it('reports a decrease as a decrease', async () => {
    mockDocking().mockResolvedValue({
      ...fixture(),
      previous: { total_stows: 40 },
    });
    const node = await dockingWidget.render(ownerCtx(), 'compact');
    const { container } = render(node as React.ReactElement);
    // 10 vs 40 is a fall. A widget that rendered the magnitude without
    // the sign would read as growth.
    expect(container.textContent).toContain('−30');
    expect(container.textContent).toContain('(−75%)');
  });

  it('names an empty predecessor instead of inventing a ratio', async () => {
    mockDocking().mockResolvedValue({
      ...fixture(),
      previous: { total_stows: 0 },
    });
    const node = await dockingWidget.render(ownerCtx(), 'compact');
    const { container } = render(node as React.ReactElement);
    expect(container.textContent).toContain('none in the prev 30d');
    expect(container.textContent).not.toContain('%');
  });
});

// #363 gave these tiles an empty-window state; nothing pinned that it
// actually renders. Same bug shape as the reported "routes are not
// populating": a handle whose activity predates the range got the blank
// box a brand-new account gets.
describe('dockingWidget empty window vs empty account', () => {
  beforeEach(() => { vi.clearAllMocks(); });

  it('says the window is empty rather than going blank when lifetime has stows', async () => {
    mockDocking().mockResolvedValue({
      total_stows: 0,
      by_kind: { hangar: 0, pad: 0, other: 0 },
      by_size: { small: 0, medium: 0, large: 0, xl: 0 },
      lifetime: { total_stows: 188 },
    });
    const node = await dockingWidget.render(ownerCtx(), 'compact');
    expect(node).not.toBeNull();
    const { container } = render(node as React.ReactElement);
    expect(container.textContent).toContain('188');
    expect(container.textContent).toMatch(/30d/);
    expect(container.textContent).toMatch(/widen the range/i);
  });

  it('renders nothing at all when there are no stows in any window', async () => {
    mockDocking().mockResolvedValue({
      total_stows: 0,
      by_kind: { hangar: 0, pad: 0, other: 0 },
      by_size: { small: 0, medium: 0, large: 0, xl: 0 },
      lifetime: { total_stows: 0 },
    });
    expect(await dockingWidget.render(ownerCtx(), 'compact')).toBeNull();
  });

  it('renders nothing on the "all" range when the window is empty', async () => {
    // `all` spans retention — there is nothing wider to widen to.
    mockDocking().mockResolvedValue({
      total_stows: 0,
      by_kind: { hangar: 0, pad: 0, other: 0 },
      by_size: { small: 0, medium: 0, large: 0, xl: 0 },
      lifetime: { total_stows: 188 },
    });
    expect(await dockingWidget.render(ownerCtx(true, 'all'), 'compact')).toBeNull();
  });
});
