import { describe, it, expect, vi, beforeEach } from 'vitest';

vi.mock('@/lib/api', () => ({ getLives: vi.fn() }));

import { getLives } from '@/lib/api';
import { livesWidget } from '../lives';
import { defineWidget } from './defineWidget';
import { LOAD_FAILED, isLoadFailure, hasNoData } from './loadResult';
import { DEFAULT_SHARE_SCOPES, type ViewerCtx } from '../types';

function ownerCtx(): ViewerCtx {
  return {
    ownerHandle: 'alice',
    viewerHandle: 'alice',
    isOwner: true,
    token: 'tok',
    shareScopes: { ...DEFAULT_SHARE_SCOPES },
    recipientScopes: null,
    range: '30d',
  };
}

/**
 * A failed load and an empty one must be DIFFERENT answers.
 *
 * They were the same value (`null`), so a timed-out query rendered as
 * "nothing recorded yet" — which is what users holding hundreds of thousands
 * of records saw when the dashboard's ~37-call burst outran the 16-connection
 * pool. It reads as data being wiped, then loading back in once the burst
 * succeeds.
 */
describe('load failure is not emptiness', () => {
  beforeEach(() => vi.clearAllMocks());

  it('turns a thrown loader into LOAD_FAILED rather than a rejection', async () => {
    const w = defineWidget<{ n: number }>({
      id: 'facts',
      eyebrow: 'Facts',
      visibility: 'owner',
      load: async () => {
        throw new Error('pool timeout');
      },
      body: () => null,
    });
    // Not a rejection: `Promise.allSettled` at the call site cannot tell a
    // rejected widget from one that chose to draw nothing.
    await expect(w.load!(ownerCtx())).resolves.toBe(LOAD_FAILED);
  });

  it('leaves a deliberate empty as null', async () => {
    const w = defineWidget<{ n: number }>({
      id: 'facts',
      eyebrow: 'Facts',
      visibility: 'owner',
      load: async () => null,
      body: () => null,
    });
    expect(await w.load!(ownerCtx())).toBeNull();
  });

  it('reports a real widget’s failed fetch as a failure', async () => {
    (getLives as ReturnType<typeof vi.fn>).mockRejectedValue(new Error('boom'));
    const out = await livesWidget.load!(ownerCtx());
    expect(isLoadFailure(out), 'a rejected fetch must not read as an empty account').toBe(
      true,
    );
  });

  it('reports a genuinely empty account as empty', async () => {
    (getLives as ReturnType<typeof vi.fn>).mockResolvedValue({
      total_lives: 0,
      deaths: 0,
      deaths_inferred: 0,
      mean_life_secs: null,
      longest_life_secs: null,
      sessions: 0,
      deaths_per_session: null,
      lives_ended_by_crash: 0,
      recent_lives: [],
    });
    const out = await livesWidget.load!(ownerCtx());
    expect(out).toBeNull();
    expect(isLoadFailure(out)).toBe(false);
  });

  it('hasNoData covers both, so only renderers must branch', () => {
    expect(hasNoData(null)).toBe(true);
    expect(hasNoData(LOAD_FAILED)).toBe(true);
    expect(hasNoData({ n: 1 })).toBe(false);
  });
});
