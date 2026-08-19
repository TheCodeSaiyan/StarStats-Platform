import { describe, expect, it } from 'vitest';

import {
  findEntityInBundles,
  isCosmeticItemPort,
  isNonLinkableItemClass,
  resolveReferenceEntry,
  type AllReferenceBundles,
  type CategoryBundle,
  type ReferenceCatalog,
  type ReferenceCategory,
  type ReferenceEntry,
} from './reference';

function refEntry(
  category: ReferenceCategory,
  class_name: string,
  display_name: string,
  slug: string | null = null,
): ReferenceEntry {
  return { category, class_name, display_name, slug, summary: { category } };
}

function makeCatalog(entries: ReferenceEntry[]): ReferenceCatalog {
  const m = new Map<string, ReferenceEntry>();
  for (const e of entries) m.set(e.class_name.toLowerCase(), e);
  return m;
}

function bundle(entries: ReferenceEntry[]): CategoryBundle {
  return { map: new Map(), catalog: makeCatalog(entries), list: entries };
}

describe('resolveReferenceEntry — tray mirror', () => {
  const vehicles = makeCatalog([
    refEntry('vehicle', 'ARGO_MOLE', 'ARGO MOLE', 'argo-mole'),
  ]);

  it('strips _Teach loaner suffix', () => {
    expect(
      resolveReferenceEntry('vehicle', 'ARGO_MOLE_Teach', vehicles)?.slug,
    ).toBe('argo-mole');
  });

  it('resolves exact + case-insensitive', () => {
    expect(resolveReferenceEntry('vehicle', 'argo_mole', vehicles)?.slug).toBe(
      'argo-mole',
    );
  });

  it('filters avatar/structural item noise', () => {
    const items = makeCatalog([
      refEntry('item', 'grin_multitool_01', 'Greycat Multi-Tool', 'multitool'),
    ]);
    expect(
      resolveReferenceEntry('item', 'Head_Eyelashes', items),
    ).toBeUndefined();
    expect(
      resolveReferenceEntry('item', 'grin_multitool_01', items)?.slug,
    ).toBe('multitool');
  });
});

describe('isNonLinkableItemClass / isCosmeticItemPort — tray mirror', () => {
  it('flags noise classes but not equipment', () => {
    expect(isNonLinkableItemClass('Default')).toBe(true);
    expect(isNonLinkableItemClass('Shared_Scalp_Unified')).toBe(true);
    expect(isNonLinkableItemClass('grin_multitool_01')).toBe(false);
  });

  it('flags cosmetic ports but not equipment ports', () => {
    expect(isCosmeticItemPort('Hair_ItemPort')).toBe(true);
    expect(isCosmeticItemPort('weapon_attach_hand_right')).toBe(false);
    expect(isCosmeticItemPort(null)).toBe(false);
  });
});

describe('findEntityInBundles — applies noise + suffix logic', () => {
  const bundles: AllReferenceBundles = {
    vehicle: bundle([refEntry('vehicle', 'ARGO_MOLE', 'ARGO MOLE', 'argo-mole')]),
    weapon: bundle([]),
    item: bundle([
      refEntry('item', 'grin_multitool_01', 'Greycat Multi-Tool', 'multitool'),
    ]),
    location: bundle([]),
  };

  it('finds a loaner variant via suffix strip', () => {
    const hit = findEntityInBundles('ARGO_MOLE_Teach', bundles);
    expect(hit?.category).toBe('vehicle');
    expect(hit?.entry.slug).toBe('argo-mole');
  });

  it('does not bind avatar noise even though it probes the item catalog', () => {
    expect(findEntityInBundles('Head_Eyelashes', bundles)).toBeNull();
  });

  it('returns null for a genuinely unknown identifier', () => {
    expect(findEntityInBundles('NOPE_Unknown_Thing', bundles)).toBeNull();
  });
});
