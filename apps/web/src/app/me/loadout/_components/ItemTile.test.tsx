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

import { ItemTile } from './ItemTile';
import type { ResolvedItem } from '@/lib/api';

const resolvedWithImage: ResolvedItem = {
  display_name: 'Light Helmet Mk1',
  slug: 'light-helmet-mk1',
  category: 'item',
  classification: 'FPS.Armor.Helmet',
  classification_label: 'Helmet',
  has_image: true,
};

const resolvedNoImage: ResolvedItem = {
  display_name: 'Ballistic Pistol',
  slug: 'ballistic-pistol',
  category: 'weapon',
  classification: 'FPS.Weapon.Pistol',
  classification_label: 'Pistol',
  has_image: false,
};

const resolvedNoSlug: ResolvedItem = {
  display_name: 'Unknown Gadget',
  slug: null,
  category: 'item',
  classification: null,
  classification_label: null,
  has_image: false,
};

describe('ItemTile', () => {
  it('renders an img with proxied src when has_image is true', () => {
    render(
      <ItemTile cls="HEL_Light_Mk1" port="head_attach" resolved={resolvedWithImage} />,
    );
    const img = screen.getByRole('img');
    expect(img).toBeInTheDocument();
    expect(img).toHaveAttribute('src', '/kb/media/item/HEL_Light_Mk1/0');
  });

  it('does not render an img when has_image is false', () => {
    render(
      <ItemTile cls="BEHR_P4AR" port="weapon_attach" resolved={resolvedNoImage} />,
    );
    expect(screen.queryByRole('img')).not.toBeInTheDocument();
    expect(screen.getByText('Ballistic Pistol')).toBeInTheDocument();
  });

  it('wraps name in a Link when slug is present', () => {
    render(
      <ItemTile cls="HEL_Light_Mk1" port="head_attach" resolved={resolvedWithImage} />,
    );
    const link = screen.getByRole('link', { name: /Light Helmet Mk1/i });
    expect(link).toHaveAttribute('href', '/kb/item/light-helmet-mk1');
  });

  it('renders name as plain text when slug is null', () => {
    render(
      <ItemTile cls="UNKN_gadget_1" port="utility_attach" resolved={resolvedNoSlug} />,
    );
    expect(screen.queryByRole('link')).not.toBeInTheDocument();
    expect(screen.getByText('Unknown Gadget')).toBeInTheDocument();
  });

  it('falls back to prettified class name when no resolved is provided', () => {
    render(<ItemTile cls="HEL_Light_MK1_23" port="head_attach" />);
    // prettify strips trailing _23, splits _, capitalises first char only (inner case preserved)
    expect(screen.getByText('HEL Light MK1')).toBeInTheDocument();
  });
});
