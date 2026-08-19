/**
 * Invariant tests for the admin nav config.
 *
 * The href test is deliberately FILESYSTEM-backed. `settings-nav-config`
 * documents an equivalent nav/section invariant but its "one link per
 * configured item" test derives the expected count from the very config
 * it is checking, so config and reality can drift with a green suite —
 * which is how a `Sharing` nav entry survived pointing at an anchor that
 * had been removed. Checking hrefs against real page.tsx files on disk
 * cannot pass that way.
 */

import { describe, expect, it } from 'vitest';
import { existsSync } from 'node:fs';
import { join } from 'node:path';
import { ADMIN_NAV, ADMIN_NAV_ITEMS } from './admin-nav-config';

const APP_DIR = join(process.cwd(), 'src', 'app');

function routeFileFor(href: string): string {
  const path = href.split('?')[0]; // '/admin/submissions?status=review'
  const rel = path.replace(/^\//, ''); // 'admin/submissions'
  return join(APP_DIR, rel, 'page.tsx');
}

describe('ADMIN_NAV', () => {
  it('points every item at a route that exists on disk', () => {
    const missing = ADMIN_NAV_ITEMS.filter(
      (item) => !existsSync(routeFileFor(item.href)),
    ).map((item) => `${item.id} -> ${item.href}`);
    expect(missing).toEqual([]);
  });

  it('has no duplicate ids', () => {
    const ids = ADMIN_NAV_ITEMS.map((i) => i.id);
    expect(new Set(ids).size).toBe(ids.length);
  });

  it('has no duplicate hrefs', () => {
    const hrefs = ADMIN_NAV_ITEMS.map((i) => i.href);
    expect(new Set(hrefs).size).toBe(hrefs.length);
  });

  it('flattens in document order', () => {
    expect(ADMIN_NAV_ITEMS).toEqual(ADMIN_NAV.flatMap((c) => c.items));
  });
});
