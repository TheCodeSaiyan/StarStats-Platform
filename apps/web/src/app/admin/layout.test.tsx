import React from 'react';
import { describe, it, expect, vi, beforeEach } from 'vitest';

// `redirect` throws in the real implementation, so the mock does too —
// otherwise a failed gate falls through into code that assumes it
// passed. Same convention as the /admin page tests.
vi.mock('next/navigation', () => ({
  redirect: vi.fn((url: string) => {
    throw new Error(`REDIRECT:${url}`);
  }),
}));

vi.mock('@/lib/session', () => ({ getSession: vi.fn() }));
vi.mock('@/lib/api', () => ({ getMe: vi.fn() }));
vi.mock('@/lib/theme', () => ({ getTheme: vi.fn().mockResolvedValue('terra') }));
vi.mock('@/app/me/_projection/actions', () => ({
  setCalibrationAction: vi.fn(),
}));
vi.mock('./_projection/ConsoleShell', () => ({
  ConsoleShell: () => null,
}));

import { getSession } from '@/lib/session';
import { getMe } from '@/lib/api';
import AdminLayout from './layout';

const mockGetSession = getSession as ReturnType<typeof vi.fn>;
const mockGetMe = getMe as ReturnType<typeof vi.fn>;

/** A session cookie claiming staff. Unsigned, so anyone can mint this. */
function forgedStaffCookie() {
  mockGetSession.mockResolvedValue({
    token: 'a-real-but-non-staff-token',
    userId: 'u1',
    claimedHandle: 'nobody',
    emailVerified: true,
    staffRoles: ['admin'],
  });
}

beforeEach(() => {
  vi.clearAllMocks();
});

describe('AdminLayout staff gate', () => {
  it('refuses a cookie that claims staff when the token does not', async () => {
    forgedStaffCookie();
    // The API is the authority, and it says this token holds no roles.
    mockGetMe.mockResolvedValue({ staff_roles: [] });

    await expect(AdminLayout({ children: null })).rejects.toThrow(
      'REDIRECT:/me',
    );
  });

  it('fails closed when the role lookup errors', async () => {
    forgedStaffCookie();
    mockGetMe.mockRejectedValue(new Error('api down'));

    await expect(AdminLayout({ children: null })).rejects.toThrow(
      'REDIRECT:/auth/login?next=/admin',
    );
  });

  it('admits a token the API vouches for', async () => {
    mockGetSession.mockResolvedValue({
      token: 'genuine-moderator-token',
      userId: 'u2',
      claimedHandle: 'mod',
      emailVerified: true,
      staffRoles: [],
    });
    mockGetMe.mockResolvedValue({ staff_roles: ['moderator'] });

    await expect(AdminLayout({ children: null })).resolves.toBeTruthy();
  });
});
