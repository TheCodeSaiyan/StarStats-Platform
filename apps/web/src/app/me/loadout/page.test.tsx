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
    render(await LoadoutPage({ searchParams: Promise.resolve({}) }));
    expect(screen.getByText('Light Helmet')).toBeInTheDocument();
  });

  it('renders the pistol in a Weapons gear group', async () => {
    render(await LoadoutPage({ searchParams: Promise.resolve({}) }));
    expect(screen.getByText('Weapons')).toBeInTheDocument();
    expect(screen.getByText('Ballistic Pistol')).toBeInTheDocument();
  });

  it('does not render the eyeball (excluded port)', async () => {
    render(await LoadoutPage({ searchParams: Promise.resolve({}) }));
    expect(screen.queryByText('Some Eye')).not.toBeInTheDocument();
  });

  it('shows when the snapshot was captured', async () => {
    // An undated kit reads as "now". The live bug showed a months-old
    // loadout with nothing on the page to reveal its age.
    render(await LoadoutPage({ searchParams: Promise.resolve({}) }));
    expect(screen.getByText(/Restored 24 Jun 2026/)).toBeInTheDocument();
  });

  it('offers the other snapshots, and shows the newest complete restore by default', async () => {
    const older = {
      id: 0,
      event_type: 'burst_summary',
      event_timestamp: '2026-01-01T00:00:00Z',
      payload: {
        kind: 'loadout_restore',
        items: [
          { class: 'GRIN_Light_Helmet', port: 'Armor_Undersuit', category: 'item' },
          { class: 'BEHR_P4AR', port: 'weapon_attach_0', category: 'weapon' },
          { class: 'SOME_Eye', port: EYEBALL_PORT, category: 'item' },
          { class: 'EXTRA_A', port: 'weapon_attach_1', category: 'weapon' },
          { class: 'EXTRA_B', port: 'weapon_attach_2', category: 'weapon' },
        ],
      },
    };
    // Both are full restores; the NEWER one is smaller, which is exactly the
    // shape the old "most items wins" rule got wrong — it pinned the page to
    // `older` forever.
    const newer = {
      ...burstEvent,
      payload: {
        kind: 'loadout_restore',
        items: [
          { class: 'GRIN_Light_Helmet', port: 'Armor_Undersuit', category: 'item' },
          { class: 'BEHR_P4AR', port: 'weapon_attach_0', category: 'weapon' },
        ],
      },
    };
    mockListEvents.mockResolvedValue({ events: [newer, older], next_after: null });
    render(await LoadoutPage({ searchParams: Promise.resolve({}) }));

    expect(screen.getByText('Snapshots')).toBeInTheDocument();
    expect(screen.getByText('2 recorded')).toBeInTheDocument();
    // Both reachable, and the shown one is marked.
    const current = screen.getByText(/24 Jun 2026/, { selector: 'a' });
    expect(current).toHaveAttribute('aria-current', 'true');
    expect(screen.getByText(/1 Jan 2026/, { selector: 'a' })).toHaveAttribute(
      'href',
      '/me/loadout?snapshot=2026-01-01T00%3A00%3A00Z',
    );
  });

  it('honours an explicitly requested snapshot', async () => {
    const older = {
      id: 0,
      event_type: 'burst_summary',
      event_timestamp: '2026-01-01T00:00:00Z',
      payload: {
        kind: 'loadout_restore',
        items: [{ class: 'BEHR_P4AR', port: 'Armor_Undersuit', category: 'weapon' }],
      },
    };
    mockListEvents.mockResolvedValue({ events: [burstEvent, older], next_after: null });
    render(
      await LoadoutPage({
        searchParams: Promise.resolve({ snapshot: '2026-01-01T00:00:00Z' }),
      }),
    );
    expect(screen.getByText(/Restored 1 Jan 2026/)).toBeInTheDocument();
  });

  it('fills the paperdoll from the PORT when the catalogue has never heard of the armour', async () => {
    // Verbatim from a real CDS Combat Superheavy restore. The reference
    // catalogue contains cds_combat_heavy_* but nothing "superheavy", so
    // resolveReferenceItems returns NOTHING for these classes: no display
    // name, no classification. Every piece then fell through to the carried
    // "Other" bucket and the paperdoll rendered as six empty outlines, while
    // the gear was sitting right there in the payload with correct ports.
    const superheavy = {
      id: 9,
      event_type: 'burst_summary',
      event_timestamp: '2026-09-04T19:38:54.778Z',
      payload: {
        kind: 'loadout_restore',
        items: [
          { class: 'cds_combat_superheavy_helmet_01_04_01', port: 'Armor_Helmet', category: 'item' },
          { class: 'cds_combat_superheavy_suit_01_04_01', port: 'Armor_Undersuit', category: 'item' },
          { class: 'cds_combat_superheavy_arms_01_04_01', port: 'Armor_Arms', category: 'item' },
          { class: 'cds_combat_superheavy_legs_01_04_01', port: 'Armor_Legs', category: 'item' },
          { class: 'cds_combat_superheavy_backpack_01_04_01', port: 'backpack', category: 'item' },
        ],
      },
    };
    mockListEvents.mockResolvedValue({ events: [superheavy], next_after: null });
    mockResolveReferenceItems.mockResolvedValue({});

    const { container } = render(
      await LoadoutPage({ searchParams: Promise.resolve({}) }),
    );

    // Assert on the SLOTS, not on the text: the names render either way
    // (they'd just be sitting in "Other"), so text presence proves nothing.
    for (const slot of ['head', 'undersuit', 'arms', 'legs', 'back']) {
      expect(
        container.querySelector(`.hp-slot--${slot}:not(.hp-slot--empty)`),
        `the ${slot} slot must be filled from its port`,
      ).not.toBeNull();
    }
    // Nothing armour-shaped should be left in the carried buckets.
    expect(screen.queryByText('Other')).not.toBeInTheDocument();
  });

  it('renders a no-loadout message when no burst event is found', async () => {
    mockListEvents.mockResolvedValue({ events: [], next_after: null });
    render(await LoadoutPage({ searchParams: Promise.resolve({}) }));
    expect(screen.getByText(/no loadout snapshot/i)).toBeInTheDocument();
  });
});
