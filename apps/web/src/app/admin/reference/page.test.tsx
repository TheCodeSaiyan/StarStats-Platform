import React from 'react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen } from '@testing-library/react';

// `redirect` throws in the real implementation, so the mock throws too —
// otherwise a signed-out render falls through into code that assumes a
// session. Same convention as the other admin page tests.
vi.mock('next/navigation', () => ({
  redirect: vi.fn((url: string) => {
    throw new Error(`REDIRECT:${url}`);
  }),
}));

vi.mock('@/lib/session', () => ({
  getSession: vi.fn(),
}));

vi.mock('@/lib/logger', () => ({
  logger: { error: vi.fn(), info: vi.fn(), warn: vi.fn() },
}));

vi.mock('@/lib/api', () => ({
  getAdminReferenceCategories: vi.fn(),
  triggerReferenceSync: vi.fn(),
  ApiCallError: class ApiCallError extends Error {
    status: number;
    constructor(status: number, message: string) {
      super(message);
      this.status = status;
    }
  },
}));

import { getSession } from '@/lib/session';
import { getAdminReferenceCategories } from '@/lib/api';
import type { AdminReferenceCategoryDto } from '@/lib/api';
import AdminReferencePage from './page';

const mockGetSession = getSession as ReturnType<typeof vi.fn>;
const mockGetCategories = getAdminReferenceCategories as ReturnType<
  typeof vi.fn
>;

function makeCategory(
  overrides: Partial<AdminReferenceCategoryDto> = {},
): AdminReferenceCategoryDto {
  return {
    category: 'vehicle',
    entry_count: 1200,
    latest_updated_at: '2026-07-31T00:00:00Z',
    ...overrides,
  } as AdminReferenceCategoryDto;
}

async function renderPage(sync?: string) {
  const ui = await AdminReferencePage({
    searchParams: Promise.resolve(sync ? { sync } : {}),
  });
  return render(<>{ui}</>);
}

beforeEach(() => {
  vi.clearAllMocks();
  mockGetSession.mockResolvedValue({ token: 't', staffRoles: ['moderator'] });
  mockGetCategories.mockResolvedValue({ categories: [makeCategory()] });
});

describe('AdminReferencePage', () => {
  // The regression this guards: #354 replaced the daily poll with a
  // channel the server only reads on request. If nothing renders a
  // trigger, the worker never runs and reference data silently freezes.
  it('renders a control that triggers a sync', async () => {
    await renderPage();
    const button = screen.getByRole('button', { name: /sync now/i });
    expect(button).toBeTruthy();
    // A button outside a form submits nothing — the action wiring is
    // the part that actually reaches the server.
    expect(button.closest('form')).not.toBeNull();
  });

  it('reports a started sync', async () => {
    await renderPage('started');
    expect(screen.getByRole('status').textContent).toMatch(
      /sync started/i,
    );
  });

  it('reports an already-running sync without calling it an error', async () => {
    await renderPage('already_running');
    const status = screen.getByRole('status');
    expect(status.textContent).toMatch(/already queued or running/i);
    // 409 is a normal outcome; it must not be styled as a failure.
    expect(status.getAttribute('style')).not.toMatch(/--danger/);
  });

  it('surfaces a failed trigger as an error', async () => {
    await renderPage('unexpected');
    const status = screen.getByRole('status');
    expect(status.textContent).toMatch(/could not start the sync/i);
    expect(status.getAttribute('style')).toMatch(/--danger/);
  });

  it('renders no status banner when the page is loaded plainly', async () => {
    await renderPage();
    expect(screen.queryByRole('status')).toBeNull();
  });

  it('still renders the category counts', async () => {
    mockGetCategories.mockResolvedValue({
      categories: [makeCategory({ entry_count: 1200 })],
    });
    await renderPage();
    expect(screen.getByText('1,200')).toBeTruthy();
  });
});
