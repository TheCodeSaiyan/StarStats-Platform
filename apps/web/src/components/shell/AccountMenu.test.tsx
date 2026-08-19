import React from 'react';
import { describe, it, expect, vi } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';

vi.mock('next/link', () => ({
  default: ({ href, children, ...rest }: { href: unknown; children?: React.ReactNode; [key: string]: unknown }) => <a href={String(href)} {...(rest as Record<string, string>)}>{children}</a>,
}));
vi.mock('@/components/SupporterChip', () => ({ SupporterChip: () => null }));

import { AccountMenu } from './AccountMenu';

describe('AccountMenu', () => {
  it('is collapsed by default and opens on click', () => {
    render(<AccountMenu handle="alice" staffRoles={[]} />);
    const trigger = screen.getByRole('button', { name: /@alice/ });
    expect(trigger).toHaveAttribute('aria-expanded', 'false');
    expect(screen.queryByRole('link', { name: 'Settings' })).not.toBeInTheDocument();
    fireEvent.click(trigger);
    expect(trigger).toHaveAttribute('aria-expanded', 'true');
    expect(screen.getByRole('link', { name: 'Settings' })).toBeInTheDocument();
    expect(screen.getByRole('link', { name: 'Sign out' })).toBeInTheDocument();
  });

  it('hides Admin unless the user has a staff role', () => {
    const { rerender } = render(<AccountMenu handle="alice" staffRoles={[]} />);
    fireEvent.click(screen.getByRole('button', { name: /@alice/ }));
    expect(screen.queryByRole('link', { name: 'Admin' })).not.toBeInTheDocument();
    // Rerender with a staff role; the menu stays open (RTL preserves state)
    // and immediately reflects the new role — no second click needed.
    rerender(<AccountMenu handle="alice" staffRoles={['admin']} />);
    expect(screen.getByRole('link', { name: 'Admin' })).toBeInTheDocument();
  });

  it('links My public profile to the handle', () => {
    render(<AccountMenu handle="Alice" staffRoles={[]} />);
    fireEvent.click(screen.getByRole('button', { name: /@Alice/ }));
    expect(screen.getByRole('link', { name: 'My public profile' })).toHaveAttribute('href', '/u/Alice');
  });

  it('is a disclosure (aria-controls, no menu/menuitem roles) — M-W10', () => {
    render(<AccountMenu handle="alice" staffRoles={[]} />);
    const trigger = screen.getByRole('button', { name: /@alice/ });
    expect(trigger).toHaveAttribute('aria-controls', 'account-menu');
    expect(trigger).not.toHaveAttribute('aria-haspopup', 'menu');
    fireEvent.click(trigger);
    // The panel is a labelled nav region, not a role=menu.
    expect(
      screen.getByRole('navigation', { name: 'Account' }),
    ).toBeInTheDocument();
    expect(screen.queryByRole('menu')).not.toBeInTheDocument();
    expect(screen.queryAllByRole('menuitem')).toHaveLength(0);
  });
});
