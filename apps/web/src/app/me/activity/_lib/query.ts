import type { Route } from 'next';
import { DEFAULT_RANGE, type RangeId } from '@/lib/range';

/**
 * The `/me/activity` URL contract. Kept out of `page.tsx` because Next's
 * page type check forbids any export from a page module beyond its own
 * (`default`, `metadata`, …) — a shared constant there fails the build.
 */

/** Rows per page. The server clamps at 500. */
export const PAGE_SIZE = 100;

export interface ActivityQuery {
  range: RangeId;
  type?: string;
  all: boolean;
  before?: number;
}

/** Mirrors the server's own validation (`[a-z0-9_]{1,64}`). */
export function parseType(raw: string | undefined): string | undefined {
  return raw && /^[a-z0-9_]{1,64}$/.test(raw) ? raw : undefined;
}

export function parseSeq(raw: string | undefined): number | undefined {
  if (!raw || !/^\d{1,18}$/.test(raw)) return undefined;
  const n = Number(raw);
  return Number.isSafeInteger(n) && n > 0 ? n : undefined;
}

/** Filters only — what the range tabs must carry along on a range change. */
export function filterQuery(q: ActivityQuery): string {
  const p = new URLSearchParams();
  if (q.type) p.set('type', q.type);
  if (q.all) p.set('all', '1');
  const s = p.toString();
  return s ? `&${s}` : '';
}

export function hrefFor(q: ActivityQuery): Route {
  const p = new URLSearchParams();
  if (q.range !== DEFAULT_RANGE) p.set('range', q.range);
  if (q.type) p.set('type', q.type);
  if (q.all) p.set('all', '1');
  if (q.before !== undefined) p.set('before', String(q.before));
  const s = p.toString();
  return `/me/activity${s ? `?${s}` : ''}` as Route;
}
