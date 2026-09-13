import { describe, expect, it } from 'vitest';
import { DEFAULT_LAYOUT } from '@/lib/profile-layout';
import { PROFILE_CATALOGUE, PROFILE_ELEMENT_IDS } from './catalogue';

/**
 * The catalogue mirrors `DEFAULT_LAYOUT` by hand, because `profile-layout.ts`
 * is `server-only` and the catalogue is pulled into the client bundle by the
 * layout editor. These assertions are what makes that copy safe: add a widget
 * to the profile surface and forget the catalogue, and this fails rather than
 * the editor quietly offering a stale list.
 */
describe('profile element catalogue', () => {
  it('offers every profile widget except the ring-drawn one', () => {
    // `journey` is `kind: 'ring'` and this surface's ring carries the public
    // by_type distribution, so enabling it here could never draw anything.
    const expected = DEFAULT_LAYOUT.map((e) => e.id).filter(
      (id) => id !== 'journey',
    );
    expect(PROFILE_ELEMENT_IDS).toEqual(expected);
  });

  it('marks on-by-default exactly as the default layout does', () => {
    const expected = DEFAULT_LAYOUT.filter((e) => e.enabled).map((e) => e.id);
    const on = PROFILE_CATALOGUE.filter((e) => e.on).map((e) => e.id);
    expect(on).toEqual(expected);
  });

  it('gives every entry a name and a profile-shaped group', () => {
    for (const e of PROFILE_CATALOGUE) {
      expect(e.name, `${e.id} has no name`).toBeTruthy();
      // Not the /me groupings: this surface has no lenses and no layout-driven
      // ring, so "Lens panes" / "Centre ring" would describe nothing.
      expect(['Figures', 'Panels']).toContain(e.group);
    }
  });
});
