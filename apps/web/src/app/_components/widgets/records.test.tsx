import React from 'react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render } from '@testing-library/react';

// next/link → plain <a> so the biggest-trade KB deep-link renders in jsdom.
vi.mock('next/link', () => ({
  default: ({ href, children }: { href: string; children: React.ReactNode }) => (
    <a href={String(href)}>{children}</a>
  ),
}));

// Scoping guarantees:
//  - C2: a visitor must NOT fetch me-scoped data (getRecords /
//    getBiggestTrade) — it would blend the viewer's own records into
//    the owner's. Only the handle-scoped getSessions is safe.
//  - F9: the owner reads server-computed records (getRecords) AND the
//    server-computed biggest trade (getBiggestTrade) instead of computing
//    them from raw, fetch-capped event / commerce lists client-side.
vi.mock('@/lib/api', () => ({
  getSessions: vi.fn(),
  getBiggestTrade: vi.fn(),
  getRecords: vi.fn(),
}));

// Empty items catalog by default → biggest-trade item renders as plain
// text; the deep-link test overrides it with a populated catalog.
vi.mock('@/lib/reference', () => ({
  loadAllReferenceBundles: vi
    .fn()
    .mockResolvedValue({ catalogs: { items: new Map() } }),
}));

import { getSessions, getBiggestTrade, getRecords } from '@/lib/api';
import { loadAllReferenceBundles } from '@/lib/reference';
import type { ReferenceEntry } from '@/lib/reference-types';
import { recordsWidget } from './records';
import { DEFAULT_SHARE_SCOPES } from './types';
import type { ViewerCtx } from './types';

const asMock = (fn: unknown) => fn as ReturnType<typeof vi.fn>;

function ownerCtx(): ViewerCtx {
  return {
    ownerHandle: 'alice',
    viewerHandle: 'alice',
    isOwner: true,
    token: 'alice-tok',
    shareScopes: { ...DEFAULT_SHARE_SCOPES },
    recipientScopes: null,
    range: '30d',
  };
}

function visitorCtx(): ViewerCtx {
  return {
    ownerHandle: 'alice',
    viewerHandle: 'bob',
    isOwner: false,
    token: 'bob-tok',
    shareScopes: { ...DEFAULT_SHARE_SCOPES, records: true },
    recipientScopes: null,
    range: '30d',
  };
}

const SESSIONS = {
  sessions: [
    {
      started_at: '2026-01-01T00:00:00Z',
      ended_at: '2026-01-01T02:00:00Z',
      event_count: 10,
    },
  ],
};

const RECORDS = {
  longest_session_secs: 7200,
  busiest_session_events: 10,
  longest_survival_streak_secs: 300,
  deadliest_session_deaths: 2,
};

describe('recordsWidget scoping (C2 + F9)', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    asMock(getSessions).mockResolvedValue(SESSIONS);
    asMock(getBiggestTrade).mockResolvedValue({ quantity: null, item: null });
    asMock(getRecords).mockResolvedValue(RECORDS);
  });

  it('is available to the owner and to a permitted visitor', () => {
    expect(recordsWidget.isAvailable(ownerCtx())).toBe(true);
    expect(recordsWidget.isAvailable(visitorCtx())).toBe(true);
  });

  it('is unavailable to a visitor without the records share scope', () => {
    const denied = { ...visitorCtx(), shareScopes: { ...DEFAULT_SHARE_SCOPES } };
    expect(recordsWidget.isAvailable(denied)).toBe(false);
  });

  it('owner render reads server records (F9) + me-scoped commerce, not the capped sessions list', async () => {
    const result = await recordsWidget.render(ownerCtx(), 'expanded');
    expect(result).not.toBeNull();
    // Now range-scoped: passes the window hours (30d = 720h) for the split view.
    expect(getRecords).toHaveBeenCalledWith('alice-tok', 720);
    expect(getBiggestTrade).toHaveBeenCalled();
    // Records now come from the server aggregate, so the owner path no
    // longer fetches the capped sessions list.
    expect(getSessions).not.toHaveBeenCalled();
  });

  it('deep-links the biggest-trade item to the KB when the catalog has it', async () => {
    asMock(getBiggestTrade).mockResolvedValue({ quantity: 500, item: 'Laranite' });
    const laranite: ReferenceEntry = {
      category: 'item',
      class_name: 'Laranite',
      display_name: 'Laranite',
      slug: 'laranite',
      summary: { category: 'item' },
    };
    asMock(loadAllReferenceBundles).mockResolvedValueOnce({
      catalogs: { items: new Map([['laranite', laranite]]) },
    });

    const node = await recordsWidget.render(ownerCtx(), 'expanded');
    const { container } = render(node as React.ReactElement);
    expect(container.textContent).toContain('500 units');
    expect(container.textContent).toContain('Laranite');
    const link = container.querySelector('a[href="/kb/item/laranite"]');
    expect(link).not.toBeNull();
  });

  it('visitor render fetches ONLY the handle-scoped sessions, never me-scoped data', async () => {
    const result = await recordsWidget.render(visitorCtx(), 'expanded');
    // Sessions still render, so the card is not blank.
    expect(result).not.toBeNull();
    expect(getSessions).toHaveBeenCalledWith('bob-tok', 'alice');
    expect(getRecords).not.toHaveBeenCalled();
    expect(getBiggestTrade).not.toHaveBeenCalled();
  });
});
