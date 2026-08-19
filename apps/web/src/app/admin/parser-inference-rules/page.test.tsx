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
  getAdminInferenceRules: vi.fn(),
  publishAdminInferenceRule: vi.fn(),
  ApiCallError: class ApiCallError extends Error {
    status: number;
    constructor(status: number, message: string) {
      super(message);
      this.status = status;
    }
  },
}));

import { getSession } from '@/lib/session';
import { getAdminInferenceRules, publishAdminInferenceRule } from '@/lib/api';
import AdminParserInferenceRulesPage from './page';

const mockGetSession = getSession as ReturnType<typeof vi.fn>;
const mockGetAdminInferenceRules = getAdminInferenceRules as ReturnType<
  typeof vi.fn
>;
const mockPublishAdminInferenceRule = publishAdminInferenceRule as ReturnType<
  typeof vi.fn
>;

const rows = [
  {
    rule_id: 'combat.kill.inferred.v1',
    enabled: true,
    definition: {
      id: 'combat.kill.inferred.v1',
      trigger: { event_type: 'actor_death' },
      emits: { event_type: 'combat_kill_confirmed' },
      followups: [],
      confidence: 0.9,
      window_secs: 30,
    },
  },
  {
    rule_id: 'travel.jump.inferred.v1',
    enabled: false,
    definition: {
      id: 'travel.jump.inferred.v1',
      trigger: { event_type: 'jump_point' },
      emits: { event_type: 'travel_jump_confirmed' },
      followups: [],
      confidence: 0.75,
      window_secs: 60,
    },
  },
];

beforeEach(() => {
  vi.clearAllMocks();
  mockGetSession.mockResolvedValue({ token: 'test-token', staffRoles: ['admin'] });
  mockGetAdminInferenceRules.mockResolvedValue({ rules: rows });
  mockPublishAdminInferenceRule.mockResolvedValue({
    rule_id: rows[0].rule_id,
    enabled: true,
  });
});

describe('AdminParserInferenceRulesPage', () => {
  it('renders both rule_ids', async () => {
    render(
      await AdminParserInferenceRulesPage({
        searchParams: Promise.resolve({}),
      }),
    );
    expect(screen.getByText('combat.kill.inferred.v1')).toBeInTheDocument();
    expect(screen.getByText('travel.jump.inferred.v1')).toBeInTheDocument();
  });

  it('shows the trigger -> emits summary for each row', async () => {
    render(
      await AdminParserInferenceRulesPage({
        searchParams: Promise.resolve({}),
      }),
    );
    expect(
      screen.getByText((text) => text.includes('actor_death') && text.includes('combat_kill_confirmed')),
    ).toBeInTheDocument();
    expect(
      screen.getByText((text) => text.includes('jump_point') && text.includes('travel_jump_confirmed')),
    ).toBeInTheDocument();
  });

  it('shows Retract for the enabled row and Enable for the disabled row', async () => {
    render(
      await AdminParserInferenceRulesPage({
        searchParams: Promise.resolve({}),
      }),
    );
    expect(screen.getByRole('button', { name: /retract/i })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /enable/i })).toBeInTheDocument();
  });
});
