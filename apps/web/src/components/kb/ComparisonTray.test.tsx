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
    fireEvent.change(screen.getByRole('searchbox', { name: /add ship/i }), { target: { value: 'arr' } });
    fireEvent.click(screen.getByText('Arrow'));
    expect(onAdd).toHaveBeenCalledWith('arrow');
    // 'avenger' (anchor) must not appear as a suggestion <li>
    fireEvent.change(screen.getByRole('searchbox', { name: /add ship/i }), { target: { value: 'a' } });
    expect(screen.queryByRole('option', { name: 'Avenger Stalker' })).toBeNull();
  });

  it('offers cohort bulk-add and calls back with the selected key', () => {
    const onAddCohort = vi.fn();
    render(
      <ComparisonTray
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
    fireEvent.change(screen.getByRole('combobox', { name: /add cohort/i }), { target: { value: 'type:interceptor' } });
    expect(onAddCohort).toHaveBeenCalledWith('type:interceptor');
  });
});
