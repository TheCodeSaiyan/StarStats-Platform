/**
 * Signup page — invite gate wiring.
 *
 * The bug these guard: `invite_token` existed on the server and in the
 * generated client, but appeared NOWHERE in apps/web. The gate demanded
 * a field nothing could send, so flipping `gate_enabled` would have
 * rejected every signup — including people holding a valid invite.
 * Every piece present, the chain broken at one link.
 */
import React from 'react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render } from '@testing-library/react';

vi.mock('next/navigation', () => ({
  redirect: vi.fn((url: string) => {
    throw new Error(`REDIRECT:${url}`);
  }),
}));

vi.mock('@/lib/beta-gate', () => ({ isBetaGateOn: vi.fn() }));

vi.mock('@/lib/api', () => ({
  signup: vi.fn(),
  getMe: vi.fn(),
  ApiCallError: class ApiCallError extends Error {
    status: number;
    body: { error: string };
    constructor(status: number, error: string) {
      super(error);
      this.status = status;
      this.body = { error };
    }
  },
}));

vi.mock('@/lib/session', () => ({ setSession: vi.fn() }));
vi.mock('@/lib/logger', () => ({ logger: { info: vi.fn(), warn: vi.fn(), error: vi.fn() } }));
vi.mock('@/lib/metrics', () => ({ authAttemptsTotal: { inc: vi.fn() } }));

import { isBetaGateOn } from '@/lib/beta-gate';
import SignupPage from './page';

const mockGate = isBetaGateOn as ReturnType<typeof vi.fn>;

async function renderPage(params: Record<string, string> = {}) {
  const ui = await SignupPage({ searchParams: Promise.resolve(params) });
  return render(<>{ui}</>);
}

beforeEach(() => {
  vi.clearAllMocks();
});

describe('signup page invite field', () => {
  it('offers the waitlist instead of account creation while gated without an invite', async () => {
    mockGate.mockResolvedValue(true);
    const view = await renderPage();
    expect(view.getByRole('button', { name: /join the waitlist/i })).toBeTruthy();
    expect(view.queryByRole('button', { name: /create account/i })).toBeNull();
    expect(view.container.querySelector('input[name="invite_token"]')).toBeNull();
  });

  it('does NOT show an invite field while the gate is OFF', async () => {
    mockGate.mockResolvedValue(false);
    const { container } = await renderPage();
    // An invite box on an open signup is a barrier that does not exist.
    expect(container.querySelector('input[name="invite_token"]')).toBeNull();
  });

  it('prefills the invite from ?invite= so an emailed link is one click', async () => {
    mockGate.mockResolvedValue(true);
    const { container } = await renderPage({ invite: 'abc123' });
    const input = container.querySelector(
      'input[name="invite_token"]',
    ) as HTMLInputElement | null;
    expect(input?.value).toBe('abc123');
  });

  it('shows the beta banner only while the gate is on', async () => {
    mockGate.mockResolvedValue(true);
    const on = await renderPage();
    expect(on.container.textContent).toMatch(/invite-only/i);

    vi.clearAllMocks();
    mockGate.mockResolvedValue(false);
    const off = await renderPage();
    expect(off.container.textContent).not.toMatch(/invite-only/i);
  });
});

describe('signup page invite error copy', () => {
  beforeEach(() => mockGate.mockResolvedValue(true));

  // The server returns these as DIFFERENT errors and the difference
  // matters: telling someone whose invite is spent to "join the
  // waitlist" sends them back to a queue they already came through.
  it('distinguishes a missing invite from a spent one', async () => {
    const missing = await renderPage({ error: 'invite_required', invite: 'demo' });
    const missingText = missing.container.textContent ?? '';
    vi.clearAllMocks();
    mockGate.mockResolvedValue(true);
    const invalid = await renderPage({ error: 'invite_invalid', invite: 'demo' });
    const invalidText = invalid.container.textContent ?? '';

    expect(missingText).toMatch(/invite code from your waitlist email/i);
    expect(invalidText).toMatch(/already been used|not valid/i);
    expect(missingText).not.toBe(invalidText);
  });

  it('does not fall back to the generic message for gate errors', async () => {
    const { container } = await renderPage({ error: 'gate_unavailable', invite: 'demo' });
    expect(container.textContent).toMatch(/try again in a moment/i);
    expect(container.textContent).not.toMatch(/check the URL bar/i);
  });
});
