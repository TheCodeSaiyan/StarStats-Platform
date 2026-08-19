import React from 'react';
import { describe, it, expect, vi } from 'vitest';
import { render, screen } from '@testing-library/react';

vi.mock('next/link', () => ({
  default: ({
    href,
    children,
  }: {
    href: string;
    children: React.ReactNode;
  }) => <a href={String(href)}>{children}</a>,
}));

import { BodyOutline } from './BodyOutline';
import type { ResolvedItem } from '@/lib/api';

const helmetResolved: ResolvedItem = {
  display_name: 'Stalker Helmet',
  slug: 'stalker-helmet',
  category: 'item',
  classification: 'FPS.Armor.Helmet',
  classification_label: 'Helmet',
  has_image: false,
};

describe('BodyOutline', () => {
  it('renders an ItemTile for a filled head slot', () => {
    render(
      <BodyOutline
        slots={{ head: { cls: 'GRIN_Light_Helmet', port: 'head_attach', resolved: helmetResolved } }}
      />,
    );
    expect(screen.getByText('Stalker Helmet')).toBeInTheDocument();
  });

  it('renders a placeholder labelled "Legs" when the legs slot is absent', () => {
    render(<BodyOutline slots={{}} />);
    expect(screen.getByText('Legs')).toBeInTheDocument();
  });

  it('renders placeholders for all six slots when slots is empty', () => {
    render(<BodyOutline slots={{}} />);
    for (const label of ['Head', 'Torso', 'Legs', 'Undersuit', 'Back']) {
      expect(screen.getByText(label)).toBeInTheDocument();
    }
    // Arms appears twice (left + right side of the paperdoll)
    expect(screen.getAllByText('Arms')).toHaveLength(2);
  });

  it('renders filled slot tile instead of placeholder when slot is provided', () => {
    render(
      <BodyOutline
        slots={{ head: { cls: 'GRIN_Light_Helmet', port: 'head_attach', resolved: helmetResolved } }}
      />,
    );
    // The "Head" placeholder should NOT appear because the slot is filled
    expect(screen.queryByText('Head')).not.toBeInTheDocument();
    expect(screen.getByText('Stalker Helmet')).toBeInTheDocument();
  });
});
