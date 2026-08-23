import { describe, it, expect } from 'vitest';
import { existsSync } from 'node:fs';
import { join } from 'node:path';
import { readFileSync } from 'node:fs';

/**
 * Every docs-index entry points at a route that exists.
 *
 * A reference list of dead links is worse than no list — the whole reason the
 * index was added is that each docs route was previously a dead end, and an
 * index that sends readers to a 404 is a worse dead end.
 *
 * Checked against the app router on disk rather than by visiting fifteen pages
 * in Playwright. That is where this started, and it was slow enough to time out
 * under full-suite load; the claim is a filesystem fact, so it is checked as
 * one — the same approach `nav.test.ts` takes for `SITE_NAV`.
 *
 * The hrefs are read out of the component's source rather than imported,
 * because importing it would pull `next/link` and JSX into a plain node test
 * for no gain.
 */
const APP = join(process.cwd(), 'src', 'app');
const SOURCE = join(
  process.cwd(),
  'src',
  'components',
  'projection',
  'DocsIndex.tsx',
);

function hrefs(): string[] {
  const src = readFileSync(SOURCE, 'utf8');
  return [...src.matchAll(/\['(\/[a-z0-9/-]*)',\s*'[^']+'\]/g)].map(
    (m) => m[1],
  );
}

describe('DocsIndex', () => {
  it('lists the whole reference set', () => {
    // Four groups: Product (3), Help (4), Guides (5), Project (3).
    expect(hrefs()).toHaveLength(15);
  });

  it('every entry resolves to a real route', () => {
    const missing = hrefs().filter(
      (h) => !existsSync(join(APP, h.replace(/^\//, ''), 'page.tsx')),
    );
    expect(missing).toEqual([]);
  });

  it('has no duplicate entries', () => {
    const all = hrefs();
    expect(new Set(all).size).toBe(all.length);
  });
});
