import React from 'react';
import { describe, it, expect, vi } from 'vitest';

const redirect = vi.fn();
vi.mock('next/navigation', () => ({ redirect: (...a: unknown[]) => redirect(...a) }));

import JourneyRedirect from './page';

describe('/journey redirect stub', () => {
  it('redirects to /me', () => {
    JourneyRedirect();
    expect(redirect).toHaveBeenCalledWith('/me');
  });
});
