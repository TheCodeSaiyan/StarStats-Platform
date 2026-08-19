import { afterEach, beforeEach, describe, expect, it } from 'vitest';

import { kbCacheOpts, LARGE_STATS_CATEGORIES } from './reference';
import type { ReferenceCategory } from './reference-types';

// Snapshot + restore the env knob so a test that flips it doesn't
// poison the others. vitest doesn't isolate `process.env` per-test.
let savedDisable: string | undefined;
beforeEach(() => {
  savedDisable = process.env.STARSTATS_DISABLE_FETCH_CACHE;
  delete process.env.STARSTATS_DISABLE_FETCH_CACHE;
});
afterEach(() => {
  if (savedDisable === undefined) {
    delete process.env.STARSTATS_DISABLE_FETCH_CACHE;
  } else {
    process.env.STARSTATS_DISABLE_FETCH_CACHE = savedDisable;
  }
});

describe('LARGE_STATS_CATEGORIES', () => {
  // This set exists ONLY for /stats responses that exceed Next's 2 MB
  // per-entry data-cache limit. It used to be keyed on the category
  // LISTING, but that listing is no longer fetched at runtime (it ships
  // in a static package), so the old set's only surviving effect was
  // disabling the cache on the small per-slug DETAIL reads -- the exact
  // opposite of its intent, and the cause of a 429 storm.
  it('marks vehicle stats as oversized (measured 2.25 MB)', () => {
    expect(LARGE_STATS_CATEGORIES.has('vehicle')).toBe(true);
  });

  it('leaves other categories cacheable', () => {
    expect(LARGE_STATS_CATEGORIES.has('weapon')).toBe(false);
    expect(LARGE_STATS_CATEGORIES.has('location')).toBe(false);
  });
});

describe('kbCacheOpts', () => {
  // THE regression guard. Per-slug detail responses are small for every
  // category; caching them is what keeps a prefetch burst from turning
  // into a wave of uncached API calls and tripping the per-IP governor.
  it('caches per-slug detail reads for EVERY category, including the big ones', () => {
    const every: ReferenceCategory[] = ['item', 'vehicle', 'weapon', 'location'];
    for (const cat of every) {
      expect(kbCacheOpts('detail', cat)).toEqual({
        next: { revalidate: 3600 },
      });
    }
  });

  it('bypasses the cache only for oversized stats payloads', () => {
    expect(kbCacheOpts('stats', 'vehicle')).toEqual({ cache: 'no-store' });
  });

  it('still caches stats for categories under the limit', () => {
    for (const cat of ['weapon', 'location'] as ReferenceCategory[]) {
      expect(kbCacheOpts('stats', cat)).toEqual({ next: { revalidate: 3600 } });
    }
  });

  it('does not let an oversized stats category leak into its detail reads', () => {
    // vehicle/stats is too big to cache; vehicle/slug/* is not. Keying
    // the decision on the category rather than the endpoint is what
    // caused the original bug.
    expect(kbCacheOpts('detail', 'vehicle')).toEqual({
      next: { revalidate: 3600 },
    });
  });

  it('honours STARSTATS_DISABLE_FETCH_CACHE for every category', () => {
    process.env.STARSTATS_DISABLE_FETCH_CACHE = '1';
    const every: ReferenceCategory[] = ['vehicle', 'weapon', 'item', 'location'];
    for (const cat of every) {
      expect(kbCacheOpts('detail', cat)).toEqual({ cache: 'no-store' });
      expect(kbCacheOpts('stats', cat)).toEqual({ cache: 'no-store' });
    }
  });
});
