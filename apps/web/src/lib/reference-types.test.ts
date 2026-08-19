import { describe, expect, it } from 'vitest';

import {
  isCosmeticItemPort,
  isNonLinkableItemClass,
  placementLabel,
  resolveReferenceEntry,
  subtypeLabel,
  tierLabel,
  type LocationSummary,
  type Placement,
  type ReferenceCatalog,
  type ReferenceEntry,
} from './reference-types';

describe('tierLabel', () => {
  it('returns canonical labels for every tier', () => {
    expect(tierLabel('system')).toBe('System');
    expect(tierLabel('astronomical_object')).toBe('Astronomical');
    expect(tierLabel('landing_zone')).toBe('Landing zone');
    expect(tierLabel('space_station')).toBe('Space station');
    expect(tierLabel('landmark')).toBe('Landmark');
    expect(tierLabel('flotilla')).toBe('Flotilla');
    expect(tierLabel('naval_base')).toBe('Naval base');
    expect(tierLabel('anonymous_poi')).toBe('Point of interest');
  });
});

describe('subtypeLabel', () => {
  it('renders known multi-word subtypes with friendly casing', () => {
    expect(subtypeLabel('drug_lab')).toBe('Drug lab');
    expect(subtypeLabel('sealed_settlement')).toBe('Sealed settlement');
    expect(subtypeLabel('forward_operating_base')).toBe('FOB');
    expect(subtypeLabel('comm_array')).toBe('Comm array');
  });

  it('title-cases unknown subtype values for forward-compat', () => {
    // Wiki could add a new sub-bucket like `geothermal_vent` — the
    // renderer should produce something usable until the dictionary
    // is updated.
    expect(subtypeLabel('geothermal_vent')).toBe('Geothermal Vent');
  });
});

describe('placementLabel', () => {
  it('formats each Placement variant', () => {
    const cases: Array<[Placement, string]> = [
      [{ kind: 'on_body', body: 'Daymar' }, 'on Daymar'],
      [{ kind: 'orbits_body', body: 'Yela' }, 'orbits Yela'],
      [
        { kind: 'lagrange_point', lagrange: 1, body: 'Hurston' },
        'L1 of Hurston',
      ],
      [{ kind: 'sunward_from', body: 'Hurston' }, 'sunward from Hurston'],
      [{ kind: 'angle_from', degrees: -60, body: 'Monox' }, '-60° from Monox'],
    ];
    for (const [p, expected] of cases) {
      expect(placementLabel(p)).toBe(expected);
    }
  });
});

describe('LocationSummary backward compat', () => {
  it('accepts a Wave 1-only payload (no taxonomy fields)', () => {
    // Real wire shape for an unenriched row: only system/parent/tag/
    // classification. The new optional fields must not break narrowing.
    const summary: LocationSummary = {
      category: 'location',
      system: 'Stanton',
      parent: 'Hurston',
      tag: 'Stanton1b',
      classification: 'Moon',
    };
    expect(summary.tier).toBeUndefined();
    expect(summary.placement).toBeUndefined();
  });

  it('accepts a fully-enriched Wave 2 payload', () => {
    const summary: LocationSummary = {
      category: 'location',
      system: 'Stanton',
      parent: 'Hurston',
      tag: 'Lorville',
      classification: 'Settlement',
      tier: 'landing_zone',
      subtype: 'city',
      placement: { kind: 'on_body', body: 'Hurston' },
      operator: 'Hurston Dynamics',
    };
    expect(summary.tier).toBe('landing_zone');
    expect(summary.placement?.kind).toBe('on_body');
    // Narrowing should work on `kind`.
    if (summary.placement?.kind === 'on_body') {
      expect(summary.placement.body).toBe('Hurston');
    }
  });
});

function refEntry(
  category: ReferenceEntry['category'],
  class_name: string,
  display_name: string,
  slug: string | null = null,
): ReferenceEntry {
  return { category, class_name, display_name, slug, summary: { category } };
}

/** Catalog keyed by lowercased class_name, mirroring getCategoryBundle. */
function makeCatalog(entries: ReferenceEntry[]): ReferenceCatalog {
  const m = new Map<string, ReferenceEntry>();
  for (const e of entries) m.set(e.class_name.toLowerCase(), e);
  return m;
}

describe('resolveReferenceEntry — variant-suffix strip (workstream A)', () => {
  const vehicles = makeCatalog([
    refEntry('vehicle', 'ARGO_MOLE', 'ARGO MOLE', 'argo-mole'),
    refEntry('vehicle', 'DRAK_Vulture', 'Drake Vulture', 'drake-vulture'),
  ]);

  it('resolves an exact (and case-insensitive) class name', () => {
    expect(resolveReferenceEntry('vehicle', 'ARGO_MOLE', vehicles)?.slug).toBe(
      'argo-mole',
    );
    expect(resolveReferenceEntry('vehicle', 'argo_mole', vehicles)?.slug).toBe(
      'argo-mole',
    );
  });

  it('strips the _Teach loaner suffix to the base class', () => {
    // The two real misses found in the live tray DB (93 + 13 events).
    expect(
      resolveReferenceEntry('vehicle', 'ARGO_MOLE_Teach', vehicles)?.slug,
    ).toBe('argo-mole');
    expect(
      resolveReferenceEntry('vehicle', 'DRAK_Vulture_Teach', vehicles)?.slug,
    ).toBe('drake-vulture');
  });

  it('does not over-strip when there is no catalogued base', () => {
    expect(
      resolveReferenceEntry('vehicle', 'SOME_Unknown_Teach', vehicles),
    ).toBeUndefined();
  });

  it('returns undefined when no catalog is supplied', () => {
    expect(
      resolveReferenceEntry('vehicle', 'ARGO_MOLE', undefined),
    ).toBeUndefined();
  });
});

describe('resolveReferenceEntry — display alias matching', () => {
  it('resolves compact vehicle labels against catalog display names', () => {
    const vehicles = makeCatalog([
      refEntry('vehicle', 'ORIG_85X', '85X Limited', '85x-limited'),
      refEntry('vehicle', 'RSI_Polaris', 'Polaris', 'polaris'),
    ]);

    expect(resolveReferenceEntry('vehicle', '85X', vehicles)?.slug).toBe(
      '85x-limited',
    );
    expect(resolveReferenceEntry('vehicle', 'Polaris', vehicles)?.slug).toBe(
      'polaris',
    );
  });

  it('resolves punctuation and spacing variants for weapons and items', () => {
    const weapons = makeCatalog([
      refEntry(
        'weapon',
        'behr_rifle_ballistic_02_civilian',
        'P8-AR Rifle',
        'p8-ar-rifle',
      ),
    ]);
    const items = makeCatalog([
      refEntry(
        'item',
        'cds_legacy_armor_heavy_arms_01_01_12',
        'ADP Arms Black',
        'adp-arms-black',
      ),
    ]);

    expect(resolveReferenceEntry('weapon', 'P8 AR Rifle', weapons)?.slug).toBe(
      'p8-ar-rifle',
    );
    expect(
      resolveReferenceEntry('item', 'ADP Arms Black', items)?.slug,
    ).toBe('adp-arms-black');
  });

  it('resolves known location label variants and routed jump points', () => {
    const locations = makeCatalog([
      refEntry('location', 'grim-hex', 'Grim HEX', 'grim-hex'),
      refEntry(
        'location',
        'stanton-magnus-jump-point',
        'Stanton - Magnus Jump Point',
        'stanton-magnus-jump-point',
      ),
      refEntry(
        'location',
        'stanton-pyro-jump-point',
        'Stanton-Pyro Jump Point',
        'stanton-pyro-jump-point',
      ),
      refEntry(
        'location',
        'nyx-castra-jump-point',
        'Nyx - Castra Jump Point',
        'nyx-castra-jump-point',
      ),
    ]);

    expect(resolveReferenceEntry('location', 'GrimHEX', locations)?.slug).toBe(
      'grim-hex',
    );
    expect(
      resolveReferenceEntry(
        'location',
        'Stanton ↔ Magnus · Jump Point 1',
        locations,
      )?.slug,
    ).toBe('stanton-magnus-jump-point');
    expect(
      resolveReferenceEntry(
        'location',
        'Pyro ↔ Stanton · Jump Point 1',
        locations,
      )?.slug,
    ).toBe('stanton-pyro-jump-point');
    expect(
      resolveReferenceEntry(
        'location',
        'Nyx ↔ Castra · Jump Point 1',
        locations,
      )?.slug,
    ).toBe('nyx-castra-jump-point');
  });
});

describe('isNonLinkableItemClass — item noise filter (workstream D)', () => {
  it('flags avatar / structural / default classes', () => {
    for (const c of [
      'Default',
      'Default_LensDisplay_PU',
      'Head_Eyelashes',
      'Head_Teeth',
      'body_01_noMagicPocket',
      'Shared_Scalp_Unified',
      'PU_Protos_Head',
      'FP_Visor',
      'FPS_DefaultRadar_Lens',
    ]) {
      expect(isNonLinkableItemClass(c), c).toBe(true);
    }
  });

  it('does NOT flag genuine equipment', () => {
    for (const c of [
      'grin_multitool_01',
      'klwe_pistol_energy_01_mag',
      'crlf_consumable_healing_01',
      'behr_gren_frag_01',
    ]) {
      expect(isNonLinkableItemClass(c), c).toBe(false);
    }
  });

  it('keeps noise item classes from resolving (renders plain text)', () => {
    const items = makeCatalog([
      refEntry('item', 'Head_Eyelashes', 'Eyelashes', 'eyelashes'),
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

describe('isCosmeticItemPort', () => {
  it('flags avatar / structural ports, not equipment ports', () => {
    for (const p of [
      'Eyes_ItemPort',
      'Hair_ItemPort',
      'Eyelashes_ItemPort',
      'Body_ItemPort',
    ]) {
      expect(isCosmeticItemPort(p), p).toBe(true);
    }
    for (const p of [
      'weapon_attach_hand_right',
      'magazine_attach',
      'Armor_Helmet',
      'utility_attach_1',
    ]) {
      expect(isCosmeticItemPort(p), p).toBe(false);
    }
  });

  it('handles null / undefined', () => {
    expect(isCosmeticItemPort(null)).toBe(false);
    expect(isCosmeticItemPort(undefined)).toBe(false);
  });
});
