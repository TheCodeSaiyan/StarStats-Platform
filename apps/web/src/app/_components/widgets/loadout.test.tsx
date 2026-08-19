import React from 'react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen } from '@testing-library/react';

// Mock next/link so it renders a plain <a> in jsdom
vi.mock('next/link', () => ({
  default: ({
    href,
    children,
  }: {
    href: string;
    children: React.ReactNode;
  }) => <a href={String(href)}>{children}</a>,
}));

vi.mock('@/lib/api', () => ({
  listEvents: vi.fn(),
  resolveReferenceItems: vi.fn(),
  getLoadoutActivity: vi.fn(),
}));

import {
  listEvents,
  resolveReferenceItems,
  getLoadoutActivity,
} from '@/lib/api';
import { loadoutWidget } from './loadout';
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

// Burst event with a mix of visible items and an excluded anatomy port.
const BURST_EVENT = {
  id: 1,
  event_type: 'burst_summary',
  event_timestamp: '2026-06-24T00:00:00Z',
  payload: {
    kind: 'loadout_restore',
    items: [
      // visible item
      { class: 'GRIN_Light_Helmet', port: 'head_attach', category: 'item' },
      // visible item
      { class: 'BEHR_P4AR', port: 'weapon_attach_0', category: 'weapon' },
      // excluded anatomy port — must NOT count
      { class: 'SomeThing', port: 'eyes_itemport', category: 'item' },
    ],
  },
};

describe('loadoutWidget', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('is NOT range-aware (loadout is a snapshot, not a time series)', () => {
    expect(loadoutWidget.rangeAware).toBe(false);
  });

  it('fetches burst_summary events without a since filter', async () => {
    (listEvents as ReturnType<typeof vi.fn>).mockResolvedValue({ events: [] });
    (resolveReferenceItems as ReturnType<typeof vi.fn>).mockResolvedValue({});

    await loadoutWidget.render(ownerCtx('30d'), 'compact');

    expect(listEvents).toHaveBeenCalledWith(
      'tok',
      expect.objectContaining({ event_type: 'burst_summary' }),
    );
    const call = (listEvents as ReturnType<typeof vi.fn>).mock.calls[0] as [
      string,
      { since?: string; event_type?: string },
    ];
    expect(call[1].since).toBeUndefined();
  });

  it('returns null when there are no burst events (tile auto-collapses)', async () => {
    (listEvents as ReturnType<typeof vi.fn>).mockResolvedValue({ events: [] });
    (resolveReferenceItems as ReturnType<typeof vi.fn>).mockResolvedValue({});

    const node = await loadoutWidget.render(ownerCtx('30d'), 'compact');

    expect(node).toBeNull();
  });

  it('shows post-filter item count (excluded ports not counted)', async () => {
    (listEvents as ReturnType<typeof vi.fn>).mockResolvedValue({ events: [BURST_EVENT] });
    // resolveReferenceItems returns a friendly name for one class
    (resolveReferenceItems as ReturnType<typeof vi.fn>).mockResolvedValue({
      GRIN_Light_Helmet: { display_name: 'Light Helmet', slug: null, category: 'item', classification: null, classification_label: null, has_image: false },
    });

    const node = await loadoutWidget.render(ownerCtx('30d'), 'compact');
    const { container } = render(node as React.ReactElement);

    // 3 total items, 1 excluded (eyes_itemport) → 2 visible
    expect(container.textContent).toContain('2');
  });

  it('shows a resolved friendly name in the preview', async () => {
    (listEvents as ReturnType<typeof vi.fn>).mockResolvedValue({ events: [BURST_EVENT] });
    (resolveReferenceItems as ReturnType<typeof vi.fn>).mockResolvedValue({
      GRIN_Light_Helmet: { display_name: 'Light Helmet', slug: null, category: 'item', classification: null, classification_label: null, has_image: false },
    });

    const node = await loadoutWidget.render(ownerCtx('30d'), 'compact');
    render(node as React.ReactElement);

    expect(screen.getByText(/Light Helmet/i)).toBeInTheDocument();
  });

  it('renders "View loadout" link pointing to /me/loadout', async () => {
    (listEvents as ReturnType<typeof vi.fn>).mockResolvedValue({ events: [BURST_EVENT] });
    (resolveReferenceItems as ReturnType<typeof vi.fn>).mockResolvedValue({});

    const node = await loadoutWidget.render(ownerCtx('30d'), 'compact');
    render(node as React.ReactElement);

    const link = screen.getByRole('link', { name: /view loadout/i });
    expect(link).toBeInTheDocument();
    expect(link).toHaveAttribute('href', '/me/loadout');
  });

  it('falls back to prettified name when resolve returns empty', async () => {
    (listEvents as ReturnType<typeof vi.fn>).mockResolvedValue({ events: [BURST_EVENT] });
    (resolveReferenceItems as ReturnType<typeof vi.fn>).mockResolvedValue({});

    const node = await loadoutWidget.render(ownerCtx('30d'), 'compact');
    const { container } = render(node as React.ReactElement);

    // GRIN_Light_Helmet → prettify → "GRIN Light Helmet"
    // (Title-case only capitalises the first char; the rest of each word is preserved)
    expect(container.textContent).toContain('GRIN Light Helmet');
  });

  it('shows equip/store activity counts from getLoadoutActivity', async () => {
    (listEvents as ReturnType<typeof vi.fn>).mockResolvedValue({ events: [BURST_EVENT] });
    (resolveReferenceItems as ReturnType<typeof vi.fn>).mockResolvedValue({});
    (getLoadoutActivity as ReturnType<typeof vi.fn>).mockResolvedValue({
      equips: 12,
      stores: 4,
      top_items: [{ item_class: 'GRIN_Light_Helmet', count: 5 }],
    });

    const node = await loadoutWidget.render(ownerCtx('30d'), 'compact');
    const { container } = render(node as React.ReactElement);

    expect(getLoadoutActivity).toHaveBeenCalledWith('tok');
    expect(container.textContent).toContain('equips');
    expect(container.textContent).toContain('12');
    expect(container.textContent).toContain('stores');
  });
});
