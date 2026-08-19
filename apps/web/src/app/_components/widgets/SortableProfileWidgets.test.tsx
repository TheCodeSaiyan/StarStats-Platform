import React from 'react';
import { describe, it, expect, vi } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';

vi.mock('./useEditMode', () => ({
  useEditMode: () => ({ isEditing: false, setEditing: vi.fn() }),
}));
vi.mock('@/app/_actions/profile-layout', () => ({
  saveProfileLayoutAction: vi.fn(),
}));

import { SortableProfileWidgets, type RenderedWidget } from './SortableProfileWidgets';
import type { LayoutEntry } from '@/lib/api';

function setup() {
  const layout: LayoutEntry[] = [
    { id: 'combat_mission', enabled: true, size: 'compact' },
    { id: 'economy', enabled: true, size: 'compact' },
  ];
  const rendered = new Map<string, RenderedWidget>([
    [
      'combat_mission',
      {
        id: 'combat_mission',
        eyebrow: 'Combat & Missions',
        title: 'Combat & Missions',
        body: <p>combat-body</p>,
      },
    ],
    [
      'economy',
      {
        id: 'economy',
        eyebrow: 'Economy',
        title: 'Economy',
        body: <p>economy-body</p>,
      },
    ],
  ]);
  return { layout, rendered };
}

describe('SortableProfileWidgets lifetime marker', () => {
  it('shows · lifetime on non-range-aware widgets on home, not on range-aware ones', () => {
    const layout: LayoutEntry[] = [
      { id: 'orgs', enabled: true, size: 'compact' },
      { id: 'economy', enabled: true, size: 'compact' },
    ];
    const rendered = new Map<string, RenderedWidget>([
      [
        'orgs',
        {
          id: 'orgs',
          eyebrow: 'Orgs',
          title: 'Orgs',
          body: <p>orgs-body</p>,
          isRangeAware: false,
        },
      ],
      [
        'economy',
        {
          id: 'economy',
          eyebrow: 'Economy',
          title: 'Economy',
          body: <p>economy-body</p>,
          isRangeAware: true,
        },
      ],
    ]);
    render(
      <SortableProfileWidgets
        initialLayout={layout}
        rendered={rendered}
        surface="home"
        lensEnabled
      />,
    );
    // Non-range-aware: show · lifetime
    expect(screen.getByText('· lifetime')).toBeInTheDocument();
    // Range-aware: no · lifetime
    const lifetimeSpans = screen.queryAllByText('· lifetime');
    expect(lifetimeSpans).toHaveLength(1);
  });
});

describe('SortableProfileWidgets HUD tile shell', () => {
  it('renders widgets as HUD tiles inside the free grid (no .ss-card shell)', () => {
    const { layout, rendered } = setup();
    const { container } = render(
      <SortableProfileWidgets initialLayout={layout} rendered={rendered} surface="home" />,
    );
    expect(container.querySelector('.hud-freegrid')).toBeInTheDocument();
    expect(container.querySelector('.hud-tile')).toBeInTheDocument();
    expect(container.querySelector('.ss-card')).not.toBeInTheDocument();
  });

  it('gives every legacy tile a resolved grid position (backward compat)', () => {
    // A legacy {id,enabled,size} layout with NO geometry must still
    // render with a concrete grid-column / grid-row placement.
    const { layout, rendered } = setup();
    const { container } = render(
      <SortableProfileWidgets initialLayout={layout} rendered={rendered} surface="home" />,
    );
    const tiles = container.querySelectorAll<HTMLElement>('.hud-tile[data-widget-id]');
    expect(tiles.length).toBe(2);
    tiles.forEach((tile) => {
      expect(tile.style.gridColumn).toMatch(/span \d+/);
      expect(tile.style.gridRow).toMatch(/span \d+/);
    });
  });
});

describe('SortableProfileWidgets lens filter', () => {
  it('shows both widgets under All, filters to the selected lens', () => {
    const { layout, rendered } = setup();
    render(
      <SortableProfileWidgets
        initialLayout={layout}
        rendered={rendered}
        surface="home"
        lensEnabled
      />,
    );

    // All (default): both visible.
    expect(screen.getByText('combat-body')).toBeInTheDocument();
    expect(screen.getByText('economy-body')).toBeInTheDocument();

    // Pick Combat: economy filtered out, combat stays.
    fireEvent.click(screen.getByRole('button', { name: 'Combat' }));
    expect(screen.getByText('combat-body')).toBeInTheDocument();
    expect(screen.queryByText('economy-body')).not.toBeInTheDocument();
  });

  it('renders no lens chips when lensEnabled is false', () => {
    const { layout, rendered } = setup();
    render(
      <SortableProfileWidgets
        initialLayout={layout}
        rendered={rendered}
        surface="profile"
      />,
    );
    expect(screen.queryByRole('button', { name: 'Combat' })).not.toBeInTheDocument();
  });
});
