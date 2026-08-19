'use client';

import type { ReferenceCategory } from './reference-types';
import type { CompareResponse } from './kb-compare-types';

const EMPTY: CompareResponse = { entries: [] };

/**
 * Fetch numeric comparison vectors for `slugs` via the same-origin
 * `/kb/compare/{category}` proxy. Degrades to empty entries on any
 * failure (the UI shows a notice; never throws into render).
 */
export async function fetchCompareVectors(
  category: ReferenceCategory,
  slugs: string[],
): Promise<CompareResponse> {
  if (slugs.length === 0) return EMPTY;
  try {
    const resp = await fetch(`/kb/compare/${category}?slugs=${encodeURIComponent(slugs.join(','))}`, {
      method: 'GET',
    });
    if (!resp.ok) return EMPTY;
    return ((await resp.json()) as CompareResponse) ?? EMPTY;
  } catch {
    return EMPTY;
  }
}
