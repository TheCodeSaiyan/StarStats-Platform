import React from 'react';
import { describe, it, expect, vi } from 'vitest';

const redirect = vi.fn();
vi.mock('next/navigation', () => ({ redirect: (...a: unknown[]) => redirect(...a) }));

import DashboardRedirect from './page';

describe('/dashboard redirect stub', () => {
  it('redirects to /me', () => {
    DashboardRedirect();
    expect(redirect).toHaveBeenCalledWith('/me');
  });
});
