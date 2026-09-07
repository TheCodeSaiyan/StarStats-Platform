import React from 'react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen } from '@testing-library/react';

// `redirect` throws, like the real one, so a signed-out render halts.
vi.mock('next/navigation', async () => {
  const m = await import('@/test-support/next-navigation');
  return m.navigationMock();
});

vi.mock('next/link', () => ({
  default: ({
    href,
    children,
    ...rest
  }: {
    href: string;
    children: React.ReactNode;
  }) => (
    <a href={String(href)} {...rest}>
      {children}
    </a>
  ),
}));

vi.mock('@/lib/session', () => ({
  getSession: vi.fn(),
}));

vi.mock('@/lib/api', () => ({
  listEvents: vi.fn(),
  statusOf: vi.fn(() => undefined),
}));

// The static reference snapshots are a real module; only the bundle load is
// stubbed so the rows render through the plain-text fallback.
vi.mock('@/lib/reference', async (importOriginal) => {
  const original = await importOriginal<typeof import('@/lib/reference')>();
  return {
    ...original,
    loadAllReferenceBundles: vi.fn().mockResolvedValue({
      lookup: original.EMPTY_REFERENCE_LOOKUP,
      catalogs: original.EMPTY_REFERENCE_CATALOGS,
      counts: { vehicle: 0, weapon: 0, item: 0, location: 0 },
    }),
  };
});

import { redirect } from 'next/navigation';
import { getSession } from '@/lib/session';
import { listEvents } from '@/lib/api';
import ActivityPage from './page';
import { PAGE_SIZE } from './_lib/query';

const mockRedirect = redirect as unknown as ReturnType<typeof vi.fn>;
const mockGetSession = getSession as ReturnType<typeof vi.fn>;
const mockListEvents = listEvents as ReturnType<typeof vi.fn>;

const ts = '2026-09-06T14:21:46.264Z';

function ev(seq: number, event_type: string, payload: Record<string, unknown> = {}) {
  return {
    seq,
    source_offset: seq,
    log_source: 'live',
    event_type,
    event_timestamp: ts,
    payload: { type: event_type, timestamp: ts, ...payload },
  };
}

const stow = (seq: number) =>
  ev(seq, 'vehicle_stowed', {
    vehicle_id: 'v1',
    landing_area: 'LandingArea_ShipElevator_HangarMediumFront',
    landing_area_id: '1',
    zone_host_id: '2',
  });
const attach = (seq: number) =>
  ev(seq, 'attachment_received', {
    player: 'p',
    item_class: 'scu_shirt_01',
    item_id: '1',
    status: 'persistent',
    port: 'Clothing_Torso_0',
  });
const death = (seq: number) =>
  ev(seq, 'player_death', { body_class: 'body_01', zone: null });

async function renderPage(sp: Record<string, string> = {}) {
  return render(await ActivityPage({ searchParams: Promise.resolve(sp) }));
}

/** The sentence column of every log row, top to bottom. Scoped to rows
 *  because the type chips carry the same labels for the hidden types. */
function rowTexts(container: HTMLElement): string[] {
  return Array.from(container.querySelectorAll('.hp-lg .ev')).map(
    (el) => el.textContent ?? '',
  );
}

beforeEach(() => {
  vi.clearAllMocks();
  mockGetSession.mockResolvedValue({ token: 'test-token', claimedHandle: 'pilot' });
  mockListEvents.mockResolvedValue({ events: [], next_after: null });
});

describe('ActivityPage', () => {
  it('redirects signed-out visitors instead of rendering their own data', async () => {
    mockGetSession.mockResolvedValue(null);
    await expect(ActivityPage({})).rejects.toThrow('REDIRECT:/auth/login?next=/me/activity');
    expect(mockRedirect).toHaveBeenCalledWith('/auth/login?next=/me/activity');
  });

  it('renders each event as a sentence and hides instrumentation by default', async () => {
    mockListEvents.mockResolvedValue({ events: [stow(3), attach(2), death(1)], next_after: null });
    const { container } = await renderPage();

    const rows = rowTexts(container);
    expect(rows).toHaveLength(2);
    expect(rows[0]).toMatch(/stowed at/);
    expect(rows[1]).toMatch(/^Died/);
    expect(rows.join('\n')).not.toMatch(/Attached|attachment_received/);
    expect(screen.getByText(/1 hidden on this page/)).toBeInTheDocument();
    expect(screen.getByRole('link', { name: 'Show everything' }).getAttribute('href')).toContain(
      'all=1',
    );
  });

  it('shows instrumentation with ?all=1', async () => {
    mockListEvents.mockResolvedValue({ events: [stow(3), attach(2)], next_after: null });
    const { container } = await renderPage({ all: '1' });

    expect(rowTexts(container)).toHaveLength(2);
    expect(rowTexts(container)[1]).toMatch(/^Attached/);
    expect(screen.getByRole('link', { name: 'Hide instrumentation' })).toBeInTheDocument();
  });

  it('passes ?type= to the server filter and never holds back what was asked for', async () => {
    mockListEvents.mockResolvedValue({ events: [attach(2)], next_after: null });
    const { container } = await renderPage({ type: 'attachment_received' });

    expect(mockListEvents).toHaveBeenCalledWith(
      'test-token',
      expect.objectContaining({ event_type: 'attachment_received', limit: PAGE_SIZE }),
    );
    expect(rowTexts(container)).toHaveLength(1);
    expect(rowTexts(container)[0]).toMatch(/^Attached/);
  });

  it('drops a malformed ?type= rather than forwarding it', async () => {
    await renderPage({ type: 'DROP TABLE' });
    const call = mockListEvents.mock.calls[0] as [string, { event_type?: string }];
    expect(call[1].event_type).toBeUndefined();
  });

  it('scopes the fetch to the range window', async () => {
    await renderPage({ range: '24h' });
    const call = mockListEvents.mock.calls[0] as [string, { since?: string }];
    expect(call[1].since).toBeTruthy();
  });

  it('offers type chips counted from the page, linking to the filter', async () => {
    mockListEvents.mockResolvedValue({ events: [stow(3), stow(2), death(1)], next_after: null });
    await renderPage();

    const chip = screen.getByRole('link', { name: /Stowed ship\s*2/ });
    expect(chip.getAttribute('href')).toContain('type=vehicle_stowed');
  });

  it('pages older by the smallest seq on a full page, and offers Newest from a cursor', async () => {
    const full = Array.from({ length: PAGE_SIZE }, (_, i) => death(1000 - i));
    mockListEvents.mockResolvedValue({ events: full, next_after: null });
    await renderPage({ before: '2000' });

    expect(mockListEvents).toHaveBeenCalledWith(
      'test-token',
      expect.objectContaining({ before_seq: 2000 }),
    );
    const older = screen.getByRole('link', { name: 'Older →' });
    expect(older.getAttribute('href')).toContain(`before=${1000 - PAGE_SIZE + 1}`);
    expect(screen.getByRole('link', { name: '← Newest' })).toBeInTheDocument();
  });

  it('does not offer Older on a short page', async () => {
    mockListEvents.mockResolvedValue({ events: [death(1)], next_after: null });
    await renderPage();
    expect(screen.queryByRole('link', { name: 'Older →' })).toBeNull();
  });

  it('says so when the events service is down, and renders no rows', async () => {
    mockListEvents.mockRejectedValue(new Error('boom'));
    await renderPage();
    expect(screen.getByText(/Couldn.t load the event log/)).toBeInTheDocument();
  });
});
