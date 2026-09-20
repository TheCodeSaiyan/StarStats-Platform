import type { Route } from 'next';
import { DEFAULT_RANGE, type RangeId } from './range';

/**
 * Routes that read `?range=` and render `RangeTabs`.
 *
 * Kept as data rather than inferred, because the two things that go wrong are
 * opposite mistakes. Omitting a windowed route silently drops the range on
 * every hop into it — the bug this module exists to fix. Adding a route that
 * does NOT window puts a parameter in the URL implying an effect it has none,
 * which is worse than dropping it: the reader believes a filter is applied.
 *
 * `/me/loadout` and `/me/hangar` are deliberately absent. Loadout is a
 * SNAPSHOT — it picks the fullest burst rather than a window — and the hangar
 * is current state. Neither has a meaningful "last 30 days".
 *
 * `/u/[handle]` is absent too: a public profile is someone else's view and
 * inherits its own range from its own URL, not from wherever the visitor
 * happened to be standing.
 */
export const RANGE_AWARE_ROUTES = [
  '/me',
  '/me/activity',
  '/me/contracts',
  '/me/travel',
] as const;

const AWARE = new Set<string>(RANGE_AWARE_ROUTES);

/**
 * Carry the reader's selected range across a link, when the destination
 * honours one.
 *
 * The range is URL state by design — `MeProjection` notes that the tabs are
 * `<Link>`s so the back button stays correct and a view stays shareable — but
 * every link BETWEEN pages was a bare path, so the selection survived only as
 * long as you stayed put. Picking 90d on `/me` and following "see all" landed
 * on Contracts at the default 7d.
 *
 * That reads as a persistent setting and behaves as a per-page one, and it
 * hides data rather than merely annoying: a reader whose events are months old
 * sees an empty page and concludes the data is missing, not that the window
 * quietly reset underneath them.
 *
 * Deliberately NOT solved with localStorage or a cookie. That would give two
 * sources of truth and make one shared URL mean different things to different
 * people.
 */
export function withRange(href: string, range: RangeId | undefined): Route {
  if (!range || range === DEFAULT_RANGE) {
    // The default is what an absent parameter already means, so writing it
    // out buys nothing and makes every shared link noisier.
    return href as Route;
  }
  const [path, query = ''] = href.split('?', 2);
  if (!AWARE.has(path)) {
    return href as Route;
  }
  // Replace rather than append: a caller passing a href that already carries
  // a range would otherwise produce `?range=7d&range=90d`, where which one
  // wins is a parser detail nobody should have to know.
  const params = new URLSearchParams(query);
  params.set('range', range);
  return `${path}?${params.toString()}` as Route;
}
