import React from 'react';
import { describe, it, expect, vi } from 'vitest';
import { render, screen } from '@testing-library/react';

vi.mock('next/link', () => ({
  default: ({ href, children, ...rest }: { href: unknown; children?: React.ReactNode; [key: string]: unknown }) => <a href={String(href)} {...(rest as Record<string, string>)}>{children}</a>,
}));
vi.mock('next/navigation', () => ({ usePathname: () => '/me' }));

import { LeftRail } from './LeftRail';

describe('LeftRail 3 pillars', () => {
  it('renders exactly the three pillars and nothing demoted', () => {
    render(<LeftRail handle="alice" staffRoles={[]} />);
    expect(screen.getByRole('link', { name: /Me/ })).toBeInTheDocument();
    expect(screen.getByRole('link', { name: /Discover/ })).toBeInTheDocument();
    expect(screen.getByRole('link', { name: /Orgs/ })).toBeInTheDocument();
    // Demoted items are no longer in the rail (they live in the @handle menu):
    expect(screen.queryByRole('link', { name: /Settings/ })).not.toBeInTheDocument();
    expect(screen.queryByRole('link', { name: /Dashboard/ })).not.toBeInTheDocument();
    expect(screen.queryByRole('link', { name: /Journey/ })).not.toBeInTheDocument();
    expect(screen.queryByRole('link', { name: /Knowledge base/ })).not.toBeInTheDocument();
  });
});
