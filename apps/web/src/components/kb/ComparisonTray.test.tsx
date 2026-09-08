import React from 'react';
import { describe, it, expect, vi } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';
import { ComparisonTray } from './ComparisonTray';

const catalog = [
  { slug: 'avenger', display_name: 'Avenger Stalker' },
  { slug: 'gladius', display_name: 'Gladius' },
  { slug: 'arrow', display_name: 'Arrow' },
];

function setup(overrides = {}) {
  const onAdd = vi.fn();
  const onRemove = vi.fn();
  render(
    <ComparisonTray
      category="vehicle"
      anchorSlug="avenger"
      anchorName="Avenger Stalker"
      selected={[{ slug: 'gladius', name: 'Gladius', color: '#5BC8C0', onRadar: true }]}
      catalog={catalog}
      max={10}
      onAdd={onAdd}
      onRemove={onRemove}
      onToggleRadar={vi.fn()}
      {...overrides}
    />,
  );
  return { onAdd, onRemove };
}

describe('ComparisonTray', () => {
  it('renders the anchor (pinned) and selected chips + the counter', () => {
    setup();
    expect(screen.getByText('Avenger Stalker')).toBeTruthy();
    expect(screen.getByText('Gladius')).toBeTruthy();
    expect(screen.getByText('2 / 10')).toBeTruthy(); // anchor + 1
  });

  it('suggests catalog matches excluding anchor + already-selected, and adds on click', () => {
    const { onAdd } = setup();
    fireEvent.change(screen.getByRole('combobox', { name: /add vehicle/i }), { target: { value: 'arr' } });
    fireEvent.click(screen.getByRole('option', { name: 'Arrow' }));
    expect(onAdd).toHaveBeenCalledWith('arrow');
    // 'avenger' (anchor) and 'gladius' (selected) must not be offered.
    fireEvent.change(screen.getByRole('combobox', { name: /add vehicle/i }), { target: { value: 'a' } });
    expect(screen.queryByRole('option', { name: 'Avenger Stalker' })).toBeNull();
    expect(screen.queryByRole('option', { name: 'Gladius' })).toBeNull();
    // Fuzzy: a one-letter typo still finds the ship.
    fireEvent.change(screen.getByRole('combobox', { name: /add vehicle/i }), { target: { value: 'arow' } });
    expect(screen.getByRole('option', { name: 'Arrow' })).toBeInTheDocument();
  });

  it('offers cohort bulk-add and calls back with the selected key', () => {
    const onAddCohort = vi.fn();
    render(
      <ComparisonTray
        category="vehicle"
        anchorSlug="avenger"
        anchorName="Avenger Stalker"
        selected={[]}
        catalog={[]}
        max={10}
        onAdd={vi.fn()}
        onRemove={vi.fn()}
        onToggleRadar={vi.fn()}
        cohorts={[{ key: 'type:interceptor', kind: 'type', label: 'Interceptors' }]}
        onAddCohort={onAddCohort}
      />,
    );
    // The cohort list is searchable: a partial, fuzzy query narrows it.
    fireEvent.change(screen.getByRole('combobox', { name: /add cohort/i }), { target: { value: 'intrcep' } });
    fireEvent.click(screen.getByRole('option', { name: /Interceptors/ }));
    expect(onAddCohort).toHaveBeenCalledWith('type:interceptor');
  });
});

describe('ComparisonTray vocabulary', () => {
  // The whole comparison surface was written for vehicles and reused
  // verbatim, so a reader browsing weapons was asked to "Add ship…".
  it.each([
    ['vehicle', /add vehicle to comparison/i],
    ['weapon', /add weapon to comparison/i],
    ['item', /add item to comparison/i],
    ['location', /add location to comparison/i],
  ] as const)('names its own category (%s)', (category, name) => {
    render(
      <ComparisonTray
        category={category}
        anchorSlug="a"
        anchorName="Anchor"
        selected={[]}
        catalog={[]}
        max={10}
        onAdd={vi.fn()}
        onRemove={vi.fn()}
        onToggleRadar={vi.fn()}
      />,
    );
    const box = screen.getByRole('combobox', { name });
    expect(box).toBeInTheDocument();
    expect(box.getAttribute('placeholder')).not.toMatch(/ship/i);
  });
});
