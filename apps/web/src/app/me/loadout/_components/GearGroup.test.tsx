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

import { GearGroup } from './GearGroup';
import type { ResolvedItem } from '@/lib/api';

const pistolResolved: ResolvedItem = {
  display_name: 'Ballistic Pistol',
  slug: 'ballistic-pistol',
  category: 'weapon',
  classification: 'FPS.Weapon.Pistol',
  classification_label: 'Pistol',
  has_image: false,
};

describe('GearGroup', () => {
  it('renders title and one tile when items has one entry', () => {
    render(
      <GearGroup
        title="Weapons"
        items={[{ cls: 'BEHR_P4AR', port: 'weapon_attach_0', resolved: pistolResolved }]}
      />,
    );
    expect(screen.getByText('Weapons')).toBeInTheDocument();
    expect(screen.getByText('Ballistic Pistol')).toBeInTheDocument();
  });

  it('renders nothing (null) when items is empty', () => {
    const { container } = render(
      <GearGroup title="Weapons" items={[]} />,
    );
    expect(container.firstChild).toBeNull();
  });
});
