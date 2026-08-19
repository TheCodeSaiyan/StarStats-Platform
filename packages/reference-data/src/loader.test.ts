import { beforeEach, describe, expect, it } from 'vitest';

import {
  __resetConsolidatedCatalogForTests,
  loadConsolidatedCatalog,
  lookupEntry,
  referenceManifest,
} from './loader';

beforeEach(() => {
  __resetConsolidatedCatalogForTests();
});

describe('loadConsolidatedCatalog', () => {
  it('loads all four categories from the committed snapshots', () => {
    const cat = loadConsolidatedCatalog();
    expect(cat.byCategory.vehicle.length).toBeGreaterThan(0);
    expect(cat.byCategory.weapon.length).toBeGreaterThan(0);
    expect(cat.byCategory.item.length).toBeGreaterThan(0);
    expect(cat.byCategory.location.length).toBeGreaterThan(0);
  });

  it('per-category counts match the manifest', () => {
    const cat = loadConsolidatedCatalog();
    for (const key of ['vehicle', 'weapon', 'item', 'location'] as const) {
      expect(cat.byCategory[key].length).toBe(
        cat.manifest.categories[key].count,
      );
    }
  });

  it('normalises reference-dump entries with class/display/slug/summary', () => {
    const cat = loadConsolidatedCatalog();
    const avenger = cat.byClassName.get('aegs_avenger_titan');
    expect(avenger).toBeDefined();
    expect(avenger?.className).toBe('AEGS_Avenger_Titan');
    expect(avenger?.displayName).toBe('Avenger Titan');
    expect(avenger?.category).toBe('vehicle');
    expect(avenger?.slug).toBe('avenger-titan');
    // Factual summary is carried through, keyed by category.
    expect(avenger?.summary.category).toBe('vehicle');
    expect(avenger?.summary.manufacturer).toBe('Aegis Dynamics');
    // Non-location entries carry no tier.
    expect(avenger?.tier).toBeUndefined();
  });

  it('normalises a location entry, keeping its factual taxonomy', () => {
    const cat = loadConsolidatedCatalog();
    const area18 = cat.byClassName.get('area18');
    expect(area18).toBeDefined();
    expect(area18?.category).toBe('location');
    expect(area18?.displayName).toBe('Area18');
    expect(area18?.slug).toBe('area18');
    // Factual location fields survive; no prose fields present.
    expect(area18?.summary.category).toBe('location');
    expect(area18?.summary.system).toBe('Stanton');
    expect(area18?.summary.classification).toBe('Settlement');
  });

  it('carries only factual summary fields — never a prose/description key', () => {
    const cat = loadConsolidatedCatalog();
    for (const e of cat.entries) {
      expect(e.summary).not.toHaveProperty('description');
      expect(e.summary).not.toHaveProperty('summary');
    }
  });

  it('is case-insensitive on lookup', () => {
    expect(lookupEntry('ORIG_100i')?.displayName).toBe('100i');
    expect(lookupEntry('orig_100i')?.displayName).toBe('100i');
    expect(lookupEntry(undefined)).toBeUndefined();
    expect(lookupEntry('no_such_class')).toBeUndefined();
  });

  it('exposes the manifest provenance + source ids (CIG/RSI)', () => {
    const m = referenceManifest();
    expect(m.provenance).toBe('rsi-cig');
    expect(m.source_ids).toContain('rsi-cig');
    expect(m.source_ids).not.toContain('star-citizen-wiki');
  });
});
