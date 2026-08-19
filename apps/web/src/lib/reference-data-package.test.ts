import { describe, expect, it } from 'vitest';

import {
  loadConsolidatedCatalog,
  lookupEntry,
  SHIP_MATRIX_DISCLAIMER,
  SOURCES,
} from 'reference-data';
import { SHIP_MATRIX_DISCLAIMER as DISCLAIMER_SUBPATH } from 'reference-data/attribution';

/**
 * Integration guard: proves the `reference-data` workspace package
 * imports + resolves from apps/web (both the barrel and the
 * `/attribution` subpath) and that its JSON snapshots load. The
 * package has its own co-located unit tests; this one exists so the
 * mandatory web suite (`pnpm --filter web test:run` + `typecheck`)
 * exercises the cross-package boundary the app actually depends on.
 */
describe('reference-data package integration', () => {
  it('exposes the same verbatim Ship Matrix disclaimer via barrel + subpath', () => {
    expect(SHIP_MATRIX_DISCLAIMER).toBe(DISCLAIMER_SUBPATH);
    expect(SHIP_MATRIX_DISCLAIMER).toContain('© Cloud Imperium Rights LLC');
  });

  it('loads the consolidated catalogue from committed snapshots', () => {
    const catalog = loadConsolidatedCatalog();
    expect(catalog.entries.length).toBeGreaterThan(0);
    expect(lookupEntry('area18')?.category).toBe('location');
  });

  it('credits CIG/RSI only — the wiki source is gone from SOURCES', () => {
    expect(SOURCES.some((s) => s.id === 'rsi-cig')).toBe(true);
    expect(SOURCES.some((s) => s.id === 'star-citizen-wiki')).toBe(false);
  });
});
