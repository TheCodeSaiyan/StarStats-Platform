import React from 'react';
import { describe, it, expect } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { EntityLink } from './EntityLink';
import type { ReferenceCatalog, ReferenceEntry } from '@/lib/reference-types';

const stalker: ReferenceEntry = {
  category: 'vehicle',
  class_name: 'AEGS_Avenger_Stalker',
  display_name: 'Avenger Stalker',
  slug: 'aegis-avenger-stalker',
  summary: { category: 'vehicle', manufacturer: 'Aegis Dynamics', role: 'Interdiction' },
};

const catalog: ReferenceCatalog = new Map([[stalker.class_name.toLowerCase(), stalker]]);

/** Ancestors whose overflow would cut off a descendant that sticks out. */
function clippingAncestors(el: Element): Element[] {
  const out: Element[] = [];
  let a = el.parentElement;
  while (a && a !== document.documentElement) {
    const s = getComputedStyle(a);
    const clips = [s.overflow, s.overflowX, s.overflowY].some(
      (v) => v !== '' && v !== 'visible',
    );
    if (clips) out.push(a);
    a = a.parentElement;
  }
  return out;
}

describe('EntityLink hover card', () => {
  it('escapes every clipping ancestor of the link', async () => {
    const user = userEvent.setup();
    render(
      // Stand-in for `.hud-tile` (overflow:hidden) + `.hud-tile__body`
      // (overflow-y:auto): the card must not be a descendant of either.
      <div data-testid="tile" style={{ overflow: 'hidden' }}>
        <div style={{ overflowY: 'auto' }}>
          <EntityLink category="vehicle" classKey="AEGS_Avenger_Stalker" catalog={catalog} />
        </div>
      </div>,
    );
    const link = screen.getByRole('link', { name: 'Avenger Stalker' });
    await user.hover(link);

    const card = screen.getByRole('tooltip', { name: /avenger stalker details/i });
    expect(card).toHaveTextContent('Aegis Dynamics');
    expect(screen.getByTestId('tile').contains(card)).toBe(false);
    expect(clippingAncestors(card)).toEqual([]);
    expect(getComputedStyle(card).position).toBe('fixed');
  });

  it('describes the link by the card while open, and Escape dismisses it', async () => {
    const user = userEvent.setup();
    render(<EntityLink category="vehicle" classKey="AEGS_Avenger_Stalker" catalog={catalog} />);
    const link = screen.getByRole('link', { name: 'Avenger Stalker' });
    expect(link).not.toHaveAttribute('aria-describedby');

    await user.hover(link);
    const card = screen.getByRole('tooltip', { name: /avenger stalker details/i });
    expect(link.getAttribute('aria-describedby')).toBe(card.id);

    await user.keyboard('{Escape}');
    expect(screen.queryByRole('tooltip')).toBeNull();
    expect(link).not.toHaveAttribute('aria-describedby');
  });

  it('closes when the pointer leaves the link', async () => {
    const user = userEvent.setup();
    render(<EntityLink category="vehicle" classKey="AEGS_Avenger_Stalker" catalog={catalog} />);
    const link = screen.getByRole('link', { name: 'Avenger Stalker' });
    await user.hover(link);
    expect(screen.getByRole('tooltip')).toBeInTheDocument();
    await user.unhover(link);
    expect(screen.queryByRole('tooltip')).toBeNull();
  });

  it('renders no card for a link with no catalog entry', async () => {
    const user = userEvent.setup();
    render(
      <EntityLink
        category="location"
        classKey="Unknown_Place"
        resolvedSlug="some-place"
        resolvedLabel="Some Place"
      />,
    );
    const link = screen.getByRole('link', { name: 'Some Place' });
    await user.hover(link);
    expect(screen.queryByRole('tooltip')).toBeNull();
  });
});
