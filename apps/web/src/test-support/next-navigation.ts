import { vi } from 'vitest';

/**
 * `next/navigation` mock for a page test that renders a PROJECTION surface.
 *
 * The projection shell (`PaneSurface` / `MeProjection`) navigates from the
 * chrome and the crumb, so any page test that renders one now pulls in
 * `useRouter` — and a `vi.mock` factory REPLACES the module, so a partial mock
 * makes every hook in it undefined. Two page tests rediscovered that
 * independently; this is the shared answer.
 *
 * `redirect` throws, exactly as the real implementation does, so a signed-out
 * render actually halts instead of falling through into code that assumes a
 * session exists.
 *
 * Use it as:
 *
 *     vi.mock('next/navigation', async () => {
 *       const m = await import('@/test-support/next-navigation');
 *       return m.navigationMock();
 *     });
 */
export function navigationMock() {
  return {
    redirect: vi.fn((url: string) => {
      throw new Error(`REDIRECT:${url}`);
    }),
    useRouter: () => ({
      push: vi.fn(),
      replace: vi.fn(),
      refresh: vi.fn(),
      prefetch: vi.fn(),
      back: vi.fn(),
      forward: vi.fn(),
    }),
    usePathname: () => '/',
    useSearchParams: () => new URLSearchParams(),
    notFound: vi.fn(() => {
      throw new Error('NOT_FOUND');
    }),
  };
}
