import { cache } from 'react';
import 'server-only';
import { apiBase } from './api';
import { kbCacheOpts } from './reference';
import type { ReferenceCategory } from './reference-types';
import type { CategoryStats } from './kb-stats-types';

// Re-export the client-safe types + helpers so existing server-side
// importers (and the test file) that do
// `import { pickBucket, type CategoryStats, compareLabel } from './kb-stats'`
// keep working. The client bundle imports them from './kb-stats-types'
// directly to avoid pulling this `server-only` module into the browser.
export * from './kb-stats-types';

const EMPTY_STATS: CategoryStats = { groups: {} };
const STATS_FETCH_TIMEOUT_MS = 8_000;

/**
 * Fetch the cached per-category peer-group stats. Small payload (quantile
 * summaries, not raw arrays) so it's safe to cache like the other small
 * reference reads. Degrades to empty stats on any failure — the page
 * still renders, just without peer context.
 */
/** Request-deduped for the same reason as `getEntityDetail`: the stats payload
 *  for the large categories is `no-store`, so repeat calls in one render are
 *  repeat upstream requests against a per-IP limit. */
export const getCategoryStats = cache(_getCategoryStats);

async function _getCategoryStats(
  category: ReferenceCategory,
): Promise<CategoryStats> {
  try {
    const resp = await fetch(`${apiBase()}/v1/reference/${category}/stats`, {
      method: 'GET',
      signal: AbortSignal.timeout(STATS_FETCH_TIMEOUT_MS),
      // Most categories' stats are small quantile summaries, but
      // vehicle/stats has grown past Next's 2 MB cache ceiling — see
      // LARGE_STATS_CATEGORIES.
      ...kbCacheOpts('stats', category),
    });
    if (!resp.ok) {
      console.error(`reference ${category} stats returned ${resp.status} ${resp.statusText}`);
      return EMPTY_STATS;
    }
    return ((await resp.json()) as CategoryStats) ?? EMPTY_STATS;
  } catch (err) {
    console.error(`reference ${category} stats fetch failed`, err);
    return EMPTY_STATS;
  }
}
