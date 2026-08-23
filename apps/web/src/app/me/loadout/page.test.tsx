import React from 'react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen } from '@testing-library/react';

// Mock next/navigation (redirect) and next/link
vi.mock('next/navigation', async () => {
  const m = await import('@/test-support/next-navigation');
  return m.navigationMock();
});

vi.mock('next/link', () => ({
  default: ({
    href,
    children,
  }: {
    href: string;
    children: React.ReactNode;
  }) => <a href={String(href)}>{children}</a>,
}));

// Mock session
vi.mock('@/lib/session', () => ({
  getSession: vi.fn(),
}));

// Mock api functions
vi.mock('@/lib/api', () => ({
  listEvents: vi.fn(),
  resolveReferenceItems: vi.fn(),
}));

import { getSession } from '@/lib/session';
import { listEvents, resolveReferenceItems } from '@/lib/api';
import LoadoutPage from './page';
import type { ResolvedItem } from '@/lib/api';

const mockGetSession = getSession as ReturnType<typeof vi.fn>;
const mockListEvents = listEvents as ReturnType<typeof vi.fn>;
const mockResolveReferenceItems = resolveReferenceItems as ReturnType<typeof vi.fn>;

// Port that should be excluded (anatomy cosmetic)
const EYEBALL_PORT = 'eyes_itemport';

// Resolved items for the burst
const resolvedMap: Record<string, ResolvedItem> = {
  'GRIN_Light_Helmet': {
    display_name: 'Light Helmet',
    slug: 'light-helmet',
    category: 'item',
    classification: 'FPS.Armor.Helmet',
    classification_label: 'Helmet',
    has_image: false,
  },
  'BEHR_P4AR': {
    display_name: 'Ballistic Pistol',
    slug: 'ballistic-pistol',
    category: 'weapon',
    classification: 'FPS.Weapon.Pistol',
    classification_label: 'Pistol',
    has_image: false,
  },
};

const burstEvent = {
  id: 1,
  event_type: 'burst_summary',
  event_timestamp: '2026-06-24T00:00:00Z',
  payload: {
    kind: 'loadout_restore',
    items: [
      // Helmet → armor → head slot in BodyOutline
      { class: 'GRIN_Light_Helmet', port: 'head_attach', category: 'item' },
      // Pistol → weapon → Weapons gear group
      { class: 'BEHR_P4AR', port: 'weapon_attach_0', category: 'weapon' },
      // Eyeball → should be excluded (eye_slot is in EXCLUDED_PORTS)
      { class: 'SOME_Eye', port: EYEBALL_PORT, category: 'item' },
    ],
  },
};

beforeEach(() => {
  vi.clearAllMocks();
  mockGetSession.mockResolvedValue({ token: 'test-token', claimedHandle: 'pilot' });
  mockListEvents.mockResolvedValue({ events: [burstEvent], next_after: null });
  mockResolveReferenceItems.mockResolvedValue(resolvedMap);
});

describe('LoadoutPage', () => {
  it('renders the helmet in the head slot of BodyOutline', async () => {
    render(await LoadoutPage());
    expect(screen.getByText('Light Helmet')).toBeInTheDocument();
  });

  it('renders the pistol in a Weapons gear group', async () => {
    render(await LoadoutPage());
    expect(screen.getByText('Weapons')).toBeInTheDocument();
    expect(screen.getByText('Ballistic Pistol')).toBeInTheDocument();
  });

  it('does not render the eyeball (excluded port)', async () => {
    render(await LoadoutPage());
    expect(screen.queryByText('Some Eye')).not.toBeInTheDocument();
  });

  it('renders a no-loadout message when no burst event is found', async () => {
    mockListEvents.mockResolvedValue({ events: [], next_after: null });
    render(await LoadoutPage());
    expect(screen.getByText(/no loadout snapshot/i)).toBeInTheDocument();
  });
});
