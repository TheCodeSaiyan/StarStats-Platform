import { describe, it, expect } from 'vitest';
import { withRange, RANGE_AWARE_ROUTES } from './range-href';

/**
 * The selected range must survive a hop between pages.
 *
 * The range is URL state on purpose — `MeProjection` says so: the tabs are
 * `<Link>`s rather than client state so the back button stays correct and a
 * filtered view is shareable. But every link BETWEEN pages was a bare path,
 * so picking 90d on /me and clicking through to Contracts landed on the
 * default 7d, and going back landed on 7d again.
 *
 * It reads as a persistent view setting and behaves as a per-page one, which
 * is worse than either — and it hides data: a reader whose events are months
 * old sees an empty page and concludes the data is missing rather than that
 * the window silently reset.
 *
 * The fix belongs in the links, NOT in localStorage or a cookie. Persisting
 * it elsewhere would give two sources of truth and make one shared URL mean
 * different things to different people.
 */
describe('withRange', () => {
  it('carries the range to a route that honours one', () => {
    expect(withRange('/me/contracts', '90d')).toBe('/me/contracts?range=90d');
    expect(withRange('/me/activity', 'all')).toBe('/me/activity?range=all');
    expect(withRange('/me/travel', '30d')).toBe('/me/travel?range=30d');
    expect(withRange('/me', '24h')).toBe('/me?range=24h');
  });

  it('leaves a route that has no range window alone', () => {
    // Appending `?range=` here would be a lie: these pages do not window,
    // so the parameter would sit in the URL implying an effect it has none.
    expect(withRange('/me/loadout', '90d')).toBe('/me/loadout');
    expect(withRange('/me/hangar', '90d')).toBe('/me/hangar');
    expect(withRange('/sharing', '90d')).toBe('/sharing');
    expect(withRange('/settings', '90d')).toBe('/settings');
  });

  it('omits the default rather than writing it out', () => {
    // `7d` is what an absent parameter already means, so adding it buys
    // nothing and makes every shared link noisier.
    expect(withRange('/me/contracts', '7d')).toBe('/me/contracts');
  });

  it('is a no-op without a range', () => {
    expect(withRange('/me/contracts', undefined)).toBe('/me/contracts');
  });

  it('preserves an existing query string', () => {
    expect(withRange('/me/activity?type=player_death', '90d')).toBe(
      '/me/activity?type=player_death&range=90d',
    );
  });

  it('replaces a range already present rather than appending a second', () => {
    expect(withRange('/me/activity?range=7d', '90d')).toBe(
      '/me/activity?range=90d',
    );
  });

  it('lists exactly the routes that render RangeTabs', () => {
    // Guard: a new windowed page that forgets to register here silently
    // loses the range on every hop into it.
    expect([...RANGE_AWARE_ROUTES].sort()).toEqual([
      '/me',
      '/me/activity',
      '/me/contracts',
      '/me/travel',
    ]);
  });
});
