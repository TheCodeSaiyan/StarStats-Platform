'use client';

import type { ReferenceCategory } from './reference-types';
import type { CompareResponse } from './kb-compare-types';

const EMPTY: CompareResponse = { entries: [] };

/**
 * Fetch a cohort's member vectors via the same-origin `/kb/cohort/{category}`
 * proxy. Degrades to empty entries on any failure (the UI shows a notice;
 * never throws into render).
 */
export async function fetchCohortMembers(
  category: ReferenceCategory,
  key: string,
): Promise<CompareResponse> {
  if (!key) return EMPTY;
  try {
    const resp = await fetch(`/kb/cohort/${category}?key=${encodeURIComponent(key)}`, { method: 'GET' });
    if (!resp.ok) return EMPTY;
    return ((await resp.json()) as CompareResponse) ?? EMPTY;
  } catch {
    return EMPTY;
  }
}
