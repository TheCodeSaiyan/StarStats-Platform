import { beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('@/lib/session', () => ({
  getSession: vi.fn(),
}));
vi.mock('@/lib/theme', () => ({
  isTheme: (v: unknown) =>
    typeof v === 'string' && ['stanton', 'pyro', 'terra', 'nyx'].includes(v),
  setTheme: vi.fn(),
}));

import { getSession } from '@/lib/session';
import { setTheme } from '@/lib/theme';
import { persistThemeAction } from '@/app/_actions/theme';

describe('persistThemeAction', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('forwards a valid theme + session token to setTheme', async () => {
    (getSession as ReturnType<typeof vi.fn>).mockResolvedValue({
      token: 'jwt-abc',
      claimedHandle: 'TestPilot',
    });
    await persistThemeAction('pyro');
    expect(setTheme).toHaveBeenCalledWith('pyro', 'jwt-abc');
  });

  it('passes undefined bearer for an anonymous visitor', async () => {
    (getSession as ReturnType<typeof vi.fn>).mockResolvedValue(null);
    await persistThemeAction('terra');
    expect(setTheme).toHaveBeenCalledWith('terra', undefined);
  });

  it('ignores an invalid theme', async () => {
    (getSession as ReturnType<typeof vi.fn>).mockResolvedValue(null);
    // @ts-expect-error deliberately invalid
    await persistThemeAction('mango');
    expect(setTheme).not.toHaveBeenCalled();
  });
});
