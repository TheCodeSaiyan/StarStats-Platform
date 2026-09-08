import React from 'react';
import { describe, it, expect, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { SearchPicker } from './SearchPicker';

const items = [
  { key: 'avenger-stalker', label: 'Avenger Stalker' },
  { key: 'gladius', label: 'Gladius' },
  { key: 'arrow', label: 'Arrow' },
];

/** Ancestors whose overflow would cut off a descendant that sticks out. */
function clippingAncestors(el: Element): Element[] {
  const out: Element[] = [];
  let a = el.parentElement;
  while (a && a !== document.documentElement) {
    const s = getComputedStyle(a);
    if ([s.overflow, s.overflowX, s.overflowY].some((v) => v !== '' && v !== 'visible')) {
      out.push(a);
    }
    a = a.parentElement;
  }
  return out;
}

describe('SearchPicker', () => {
  it('renders the list outside every clipping ancestor, fixed to the viewport', async () => {
    const user = userEvent.setup();
    render(
      <div data-testid="card" style={{ overflow: 'hidden', transform: 'translateY(-1px)' }}>
        <SearchPicker label="Add ship" placeholder="Add…" items={items} onPick={() => {}} />
      </div>,
    );
    await user.type(screen.getByRole('combobox', { name: 'Add ship' }), 'glad');
    const list = screen.getByRole('listbox', { name: 'Add ship' });
    expect(screen.getByTestId('card').contains(list)).toBe(false);
    expect(clippingAncestors(list)).toEqual([]);
    expect(getComputedStyle(list).position).toBe('fixed');
    expect(screen.getByRole('option', { name: 'Gladius' })).toBeInTheDocument();
  });

  it('ranks fuzzily: a subsequence and a typo both find the ship', async () => {
    const user = userEvent.setup();
    render(<SearchPicker label="Add ship" placeholder="Add…" items={items} onPick={() => {}} />);
    const box = screen.getByRole('combobox', { name: 'Add ship' });
    await user.type(box, 'avstk');
    expect(screen.getAllByRole('option')[0]).toHaveTextContent('Avenger Stalker');
    await user.clear(box);
    await user.type(box, 'gladuis');
    expect(screen.getAllByRole('option')[0]).toHaveTextContent('Gladius');
  });

  it('is keyboard operable: arrows move, Enter picks, Escape closes and clears', async () => {
    const user = userEvent.setup();
    const onPick = vi.fn();
    render(<SearchPicker label="Add ship" placeholder="Add…" items={items} onPick={onPick} />);
    const box = screen.getByRole('combobox', { name: 'Add ship' });
    await user.type(box, 'a');
    const first = screen.getAllByRole('option')[0];
    expect(box).toHaveAttribute('aria-activedescendant', first.id);
    await user.keyboard('{ArrowDown}');
    const second = screen.getAllByRole('option')[1];
    expect(box).toHaveAttribute('aria-activedescendant', second.id);
    expect(second).toHaveAttribute('aria-selected', 'true');
    // For 'a': Arrow (shortest word-prefix hit) then Avenger Stalker.
    expect(second).toHaveTextContent('Avenger Stalker');
    await user.keyboard('{Enter}');
    expect(onPick).toHaveBeenCalledWith('avenger-stalker');
    // Picking clears the query and closes the list.
    expect(box).toHaveValue('');
    expect(screen.queryByRole('listbox')).toBeNull();

    await user.type(box, 'arr');
    expect(screen.getByRole('listbox')).toBeInTheDocument();
    await user.keyboard('{Escape}');
    expect(screen.queryByRole('listbox')).toBeNull();
    expect(box).toHaveValue('');
  });

  it('closes on a click outside, including when the list is portaled', async () => {
    const user = userEvent.setup();
    render(
      <>
        <button type="button">elsewhere</button>
        <SearchPicker label="Add ship" placeholder="Add…" items={items} onPick={() => {}} />
      </>,
    );
    await user.type(screen.getByRole('combobox', { name: 'Add ship' }), 'a');
    expect(screen.getByRole('listbox')).toBeInTheDocument();
    // Clicking an option must NOT count as outside.
    await user.hover(screen.getByRole('option', { name: 'Arrow' }));
    expect(screen.getByRole('listbox')).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'elsewhere' }));
    expect(screen.queryByRole('listbox')).toBeNull();
  });

  it('lists everything on focus when browsing is enabled, and picks on click', async () => {
    const user = userEvent.setup();
    const onPick = vi.fn();
    render(
      <SearchPicker
        label="Add cohort"
        placeholder="Add cohort…"
        items={[{ key: 'type:interceptor', label: 'Interceptors', hint: 'type' }]}
        onPick={onPick}
        browseWhenEmpty
      />,
    );
    await user.click(screen.getByRole('combobox', { name: 'Add cohort' }));
    await user.click(screen.getByRole('option', { name: /Interceptors/ }));
    expect(onPick).toHaveBeenCalledWith('type:interceptor');
  });

  it('shows nothing while disabled', async () => {
    const user = userEvent.setup();
    render(
      <SearchPicker label="Add ship" placeholder="Max" items={items} onPick={() => {}} disabled />,
    );
    const box = screen.getByRole('combobox', { name: 'Add ship' });
    expect(box).toBeDisabled();
    await user.click(box);
    expect(screen.queryByRole('listbox')).toBeNull();
  });
});
