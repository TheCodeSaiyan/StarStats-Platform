import React from 'react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen } from '@testing-library/react';

vi.mock('@/lib/api', () => ({
  getObjectives: vi.fn(),
}));

import { getObjectives } from '@/lib/api';
import { objectivesWidget } from './objectives';
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

describe('objectivesWidget', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('renders no outcome alongside unresolved and failed', async () => {
    (getObjectives as ReturnType<typeof vi.fn>).mockResolvedValue({
      completed: 1,
      total: 9,
      no_outcome: 1,
      unresolved: 7,
      failed: 0,
      completion_pct: 12,
    });

    const node = await objectivesWidget.render(ownerCtx('90d'), 'expanded');
    render(<>{node}</>);

    expect(screen.getByText('unresolved')).toBeDefined();
    // Query the readout containing the label, not a bare value: two
    // secondary readouts can share a count and a label-less assertion
    // wouldn't catch the labels being swapped.
    const unresolvedReadout = screen.getByText('unresolved').closest('.hud-readout');
    expect(unresolvedReadout?.textContent).toContain('7');
    const noOutcomeReadout = screen.getByText('no outcome').closest('.hud-readout');
    expect(noOutcomeReadout?.textContent).toContain('1');
  });

  it('denominates "done" by resolved objectives, matching rate — not the API total', async () => {
    // The maintainer's real numbers: 180 completed, 2 failed, 96
    // unresolved => 278 resolved, matching the reported 64% rate. The
    // API's `total` (1225) also folds in 947 objectives that never
    // resolved (`no_outcome`) — using `total` as the headline
    // denominator is the exact bug this test guards against: it would
    // render "done 180/1225" beside a "rate 64%" that describes a
    // completely different population.
    (getObjectives as ReturnType<typeof vi.fn>).mockResolvedValue({
      completed: 180,
      total: 1225,
      no_outcome: 947,
      unresolved: 96,
      failed: 2,
      completion_pct: 64,
    });

    const node = await objectivesWidget.render(ownerCtx('90d'), 'expanded');
    render(<>{node}</>);

    expect(screen.getByText('180/278')).toBeDefined();
    expect(screen.getByText('64%')).toBeDefined();
    expect(screen.queryByText('180/1225')).toBeNull();
    const noOutcomeReadout = screen.getByText('no outcome').closest('.hud-readout');
    expect(noOutcomeReadout?.textContent).toContain('947');
  });
});

describe('objectivesWidget lifetime comparison', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  /** Window figures; the lifetime twin is supplied per-test. */
  const windowFixture = {
    completed: 180,
    total: 1225,
    no_outcome: 947,
    unresolved: 96,
    failed: 2,
    completion_pct: 64,
  };

  // `all` is a real 8760h window, so the server DOES send a twin for it
  // — but that twin spans the same rows, so the note would restate the
  // headline. Suppressed client-side.
  it('renders no comparison on the "all" range even when the API sends a twin', async () => {
    (getObjectives as ReturnType<typeof vi.fn>).mockResolvedValue({
      ...windowFixture,
      lifetime: {
        completed: 900,
        failed: 20,
        unresolved: 80,
        no_outcome: 4000,
        total: 5000,
        completion_pct: 90,
      },
    });

    const { container } = render(
      <>{await objectivesWidget.render(ownerCtx('all'), 'expanded')}</>,
    );

    expect(container.querySelector('.hud-note')).toBeNull();
  });

  it('compares the window against the lifetime baseline when the API sends one', async () => {
    (getObjectives as ReturnType<typeof vi.fn>).mockResolvedValue({
      ...windowFixture,
      // resolved = 900 + 20 + 80 = 1000
      lifetime: {
        completed: 900,
        failed: 20,
        unresolved: 80,
        no_outcome: 4000,
        total: 5000,
        completion_pct: 90,
      },
    });

    const { container } = render(
      <>{await objectivesWidget.render(ownerCtx('90d'), 'expanded')}</>,
    );

    // Window headline unchanged.
    expect(screen.getByText('180/278')).toBeDefined();
    // Baseline uses the SAME completed-over-resolved denominator as the
    // headline, so the two ratios are read on one basis.
    expect(container.querySelector('.hud-note')?.textContent).toBe(
      'Lifetime — 900/1,000 done, 90%',
    );
  });

  it('renders the bare numbers with NO comparison when lifetime is absent', async () => {
    // No `lifetime` key at all — the server omits it when no window was
    // requested. An invented baseline is worse than a bare number.
    (getObjectives as ReturnType<typeof vi.fn>).mockResolvedValue({ ...windowFixture });

    const { container } = render(
      <>{await objectivesWidget.render(ownerCtx('all'), 'expanded')}</>,
    );

    expect(screen.getByText('180/278')).toBeDefined();
    expect(container.querySelector('.hud-note')).toBeNull();
    expect(container.textContent).not.toContain('Lifetime');
    // Guards the `?? 0` failure mode: a fabricated "0/0 done" baseline.
    expect(container.textContent).not.toMatch(/0\/0 done/);
  });

  it('treats an explicit null lifetime as absent', async () => {
    (getObjectives as ReturnType<typeof vi.fn>).mockResolvedValue({
      ...windowFixture,
      lifetime: null,
    });

    const { container } = render(
      <>{await objectivesWidget.render(ownerCtx('all'), 'expanded')}</>,
    );

    expect(screen.getByText('180/278')).toBeDefined();
    expect(container.textContent).not.toContain('Lifetime');
  });

  it('omits the lifetime rate rather than inventing 0% when nothing ever resolved', async () => {
    // `completion_pct` is null on the twin when nothing has EVER resolved.
    // Computing it locally would print "0%", which reads as a measured
    // career rate rather than the absence of one.
    (getObjectives as ReturnType<typeof vi.fn>).mockResolvedValue({
      ...windowFixture,
      lifetime: {
        completed: 0,
        failed: 0,
        unresolved: 0,
        no_outcome: 5000,
        total: 5000,
        completion_pct: null,
      },
    });

    const { container } = render(
      <>{await objectivesWidget.render(ownerCtx('90d'), 'expanded')}</>,
    );

    expect(container.querySelector('.hud-note')?.textContent).toBe('Lifetime — 0/0 done');
  });
  // See spend/docking: with no `previous`, the trend branch is dead code
  // under test and the lifetime fallback hides any error in it.
  it('leads with the trend on completed objectives', async () => {
    (getObjectives as ReturnType<typeof vi.fn>).mockResolvedValue({
      ...windowFixture,
      previous: { completed: 150, failed: 20, unresolved: 10, no_outcome: 5, total: 185, completion_pct: 60 },
    });

    const { container } = render(
      <>{await objectivesWidget.render(ownerCtx('90d'), 'expanded')}</>,
    );

    // windowFixture completes 180 vs 150 before.
    expect(container.querySelector('.hud-note')?.textContent).toContain('+30');
    expect(container.querySelector('.hud-note')?.textContent).toContain('vs prev 90d');
    // The RATE is deliberately not trended: two ratios with different
    // denominators would read as a change in performance when it can be
    // a change in volume. So exactly ONE trend appears — the "(+20%)"
    // above is the percentage change in the COUNT, not a second trend
    // on the 65%-vs-60% rate.
    const note = container.querySelector('.hud-note')?.textContent ?? '';
    expect(note.match(/vs prev/g)).toHaveLength(1);
  });
});

describe('objectivesWidget "All" range', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('asks for 365 days when the range is "all" — the retention limit', () => {
    // 365 days IS the hard retention limit, so "everything we have" and
    // "the last year" are the same set. An earlier version sent
    // `undefined` to mean lifetime, which promised a depth the data does
    // not have and diverged from the server's own 365-day bound.
    vi.mocked(getObjectives).mockResolvedValue({
      completed: 1, failed: 0, unresolved: 0, no_outcome: 0, total: 1, completion_pct: 100,
    } as never);

    void objectivesWidget.render(ownerCtx('all'), 'expanded');
    expect(getObjectives).toHaveBeenCalledWith('tok', 24 * 365);
  });

  it('still sends a bounded window for every other bucket', () => {
    vi.mocked(getObjectives).mockResolvedValue({
      completed: 1, failed: 0, unresolved: 0, no_outcome: 0, total: 1, completion_pct: 100,
    } as never);

    void objectivesWidget.render(ownerCtx('30d'), 'expanded');
    expect(getObjectives).toHaveBeenCalledWith('tok', 24 * 30);
  });

});

// #363 gave these tiles an empty-window state; nothing pinned that it
// actually renders. See kit/EmptyWindow for the bug it fixes.
describe('objectivesWidget empty window vs empty account', () => {
  beforeEach(() => { vi.clearAllMocks(); });

  it('says the window is empty rather than going blank when lifetime has objectives', async () => {
    (getObjectives as ReturnType<typeof vi.fn>).mockResolvedValue({
      completed: 0, failed: 0, unresolved: 0, no_outcome: 0,
      total: 0, completion_pct: 0,
      lifetime: { total: 188 },
    });
    const node = await objectivesWidget.render(ownerCtx('30d'), 'compact');
    expect(node).not.toBeNull();
    const { container } = render(<>{node}</>);
    expect(container.textContent).toContain('188');
    expect(container.textContent).toMatch(/30d/);
    expect(container.textContent).toMatch(/widen the range/i);
  });

  it('renders nothing at all when there are no objectives in any window', async () => {
    (getObjectives as ReturnType<typeof vi.fn>).mockResolvedValue({
      completed: 0, failed: 0, unresolved: 0, no_outcome: 0,
      total: 0, completion_pct: 0,
      lifetime: { total: 0 },
    });
    expect(await objectivesWidget.render(ownerCtx('30d'), 'compact')).toBeNull();
  });

  it('renders nothing on the "all" range when the window is empty', async () => {
    (getObjectives as ReturnType<typeof vi.fn>).mockResolvedValue({
      completed: 0, failed: 0, unresolved: 0, no_outcome: 0,
      total: 0, completion_pct: 0,
      lifetime: { total: 188 },
    });
    expect(await objectivesWidget.render(ownerCtx('all'), 'compact')).toBeNull();
  });
});
