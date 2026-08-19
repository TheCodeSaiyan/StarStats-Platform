import React from 'react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen } from '@testing-library/react';

// Mock next/navigation (redirect) and next/link
vi.mock('next/navigation', () => ({
  redirect: vi.fn(),
}));

vi.mock('next/link', () => ({
  default: ({
    href,
    children,
  }: {
    href: string;
    children: React.ReactNode;
  }) => <a href={String(href)}>{children}</a>,
}));

vi.mock('next/cache', () => ({
  revalidatePath: vi.fn(),
}));

// Mock session
vi.mock('@/lib/session', () => ({
  getSession: vi.fn(),
}));

// Mock api functions
vi.mock('@/lib/api', () => ({
  getAdminParserRules: vi.fn(),
  publishAdminParserRule: vi.fn(),
  ApiCallError: class ApiCallError extends Error {
    status: number;
    constructor(status: number, message: string) {
      super(message);
      this.status = status;
    }
  },
}));

import { getSession } from '@/lib/session';
import { getAdminParserRules, publishAdminParserRule } from '@/lib/api';
import AdminParserRulesPage from './page';

const mockGetSession = getSession as ReturnType<typeof vi.fn>;
const mockGetAdminParserRules = getAdminParserRules as ReturnType<typeof vi.fn>;
const mockPublishAdminParserRule = publishAdminParserRule as ReturnType<typeof vi.fn>;

const rows = [
  {
    rule_id: 'combat.kill.v1',
    event_name: 'actor_death',
    match_kind: 'event_name',
    body_regex: '',
    fields: ['killer', 'victim'],
    enabled: true,
  },
  {
    rule_id: 'travel.jump.v1',
    event_name: 'jump_point',
    match_kind: 'body_regex',
    body_regex: 'Jump.*',
    fields: ['origin', 'destination'],
    enabled: false,
  },
];

beforeEach(() => {
  vi.clearAllMocks();
  mockGetSession.mockResolvedValue({ token: 'test-token', staffRoles: ['admin'] });
  mockGetAdminParserRules.mockResolvedValue({ rules: rows });
  mockPublishAdminParserRule.mockResolvedValue({ rule: rows[0] });
});

describe('AdminParserRulesPage', () => {
  it('renders both rule_ids', async () => {
    render(await AdminParserRulesPage());
    expect(screen.getByText('combat.kill.v1')).toBeInTheDocument();
    expect(screen.getByText('travel.jump.v1')).toBeInTheDocument();
  });

  it('shows Retract for the enabled row and Enable for the disabled row', async () => {
    render(await AdminParserRulesPage());
    expect(screen.getByRole('button', { name: /retract/i })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /enable/i })).toBeInTheDocument();
  });
});
