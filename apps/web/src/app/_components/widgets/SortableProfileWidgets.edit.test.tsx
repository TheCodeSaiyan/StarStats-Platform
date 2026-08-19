import React from 'react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';

// Edit mode ON for this suite (the base suite runs with it OFF).
vi.mock('./useEditMode', () => ({
  useEditMode: () => ({ isEditing: true, setEditing: vi.fn() }),
}));

const saveMock = vi.fn();
vi.mock('@/app/_actions/profile-layout', () => ({
  saveProfileLayoutAction: (...args: unknown[]) => saveMock(...args),
}));

import { SortableProfileWidgets, type RenderedWidget } from './SortableProfileWidgets';
import type { LayoutEntry } from '@/lib/api';

function setup() {
  // combat_mission is on the dashboard; economy is available but off (→ palette).
  const layout: LayoutEntry[] = [
    { id: 'combat_mission', enabled: true, size: 'compact' },
    { id: 'economy', enabled: false, size: 'compact' },
  ];
  const rendered = new Map<string, RenderedWidget>([
    ['combat_mission', { id: 'combat_mission', eyebrow: 'Combat & Missions', title: 'Combat & Missions', body: <p>combat-body</p> }],
    ['economy', { id: 'economy', eyebrow: 'Economy', title: 'Economy', body: <p>economy-body</p> }],
  ]);
  return { layout, rendered };
}

describe('SortableProfileWidgets edit mode — palette + remove', () => {
  beforeEach(() => saveMock.mockReset());

  it('lists an available-but-off widget in the Add palette, not on the grid', () => {
    const { layout, rendered } = setup();
    render(<SortableProfileWidgets initialLayout={layout} rendered={rendered} surface="home" />);
    // Palette section present with the off widget as an addable item.
    expect(screen.getByRole('region', { name: 'Add widget' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Add Economy' })).toBeInTheDocument();
    // The off widget's body is NOT rendered on the grid.
    expect(screen.queryByText('economy-body')).not.toBeInTheDocument();
    // The on widget IS.
    expect(screen.getByText('combat-body')).toBeInTheDocument();
  });

  it('adds a widget from the palette (persists it enabled)', () => {
    const { layout, rendered } = setup();
    render(<SortableProfileWidgets initialLayout={layout} rendered={rendered} surface="home" />);
    fireEvent.click(screen.getByRole('button', { name: 'Add Economy' }));
    expect(saveMock).toHaveBeenCalledTimes(1);
    const savedLayout = saveMock.mock.calls[0][0] as LayoutEntry[];
    const economy = savedLayout.find((e) => e.id === 'economy');
    expect(economy?.enabled).toBe(true);
    // Given geometry so it lands in a concrete cell.
    expect(typeof economy?.x).toBe('number');
    expect(typeof economy?.y).toBe('number');
  });

  it('re-adding a previously-positioned widget restores its remembered spot, not x:0/bottom', () => {
    // economy is off but retains a custom position far from combat_mission.
    const layout: LayoutEntry[] = [
      { id: 'combat_mission', enabled: true, size: 'compact', x: 0, y: 0, w: 8, h: 6 },
      { id: 'economy', enabled: false, size: 'compact', x: 4, y: 12, w: 8, h: 6 },
    ];
    const rendered = new Map<string, RenderedWidget>([
      ['combat_mission', { id: 'combat_mission', eyebrow: 'Combat & Missions', title: 'Combat & Missions', body: <p>combat-body</p> }],
      ['economy', { id: 'economy', eyebrow: 'Economy', title: 'Economy', body: <p>economy-body</p> }],
    ]);
    render(<SortableProfileWidgets initialLayout={layout} rendered={rendered} surface="home" />);
    fireEvent.click(screen.getByRole('button', { name: 'Add Economy' }));
    const savedLayout = saveMock.mock.calls[0][0] as LayoutEntry[];
    const economy = savedLayout.find((e) => e.id === 'economy');
    expect(economy?.enabled).toBe(true);
    // Position is remembered (the regression was placing it at x:0, bottom row).
    expect(economy?.x).toBe(4);
    expect(economy?.y).toBe(12);
  });

  it('removes a widget from the dashboard (persists it disabled, back to palette)', () => {
    const { layout, rendered } = setup();
    render(<SortableProfileWidgets initialLayout={layout} rendered={rendered} surface="home" />);
    fireEvent.click(
      screen.getByRole('button', { name: 'Remove combat_mission from dashboard' }),
    );
    expect(saveMock).toHaveBeenCalledTimes(1);
    const savedLayout = saveMock.mock.calls[0][0] as LayoutEntry[];
    expect(savedLayout.find((e) => e.id === 'combat_mission')?.enabled).toBe(false);
  });

  it('resize nudges are bounded — Wider is disabled at the widget max width', () => {
    // heatmap max width is 24 (full grid); start it there and assert the
    // Wider control is disabled so the tile can't grow past its ceiling.
    const layout: LayoutEntry[] = [
      { id: 'heatmap', enabled: true, size: 'expanded', x: 0, y: 0, w: 24, h: 8 },
    ];
    const rendered = new Map<string, RenderedWidget>([
      ['heatmap', { id: 'heatmap', eyebrow: 'Activity', title: 'Daily activity', body: <p>heat-body</p> }],
    ]);
    render(<SortableProfileWidgets initialLayout={layout} rendered={rendered} surface="home" />);
    expect(screen.getByRole('button', { name: 'Wider' })).toBeDisabled();
    expect(screen.getByRole('button', { name: 'Narrower' })).not.toBeDisabled();
  });
});
