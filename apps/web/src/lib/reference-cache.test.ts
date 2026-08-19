import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import {
  __resetReferenceBundleCacheForTests,
  loadAllReferenceBundles,
} from './reference';

// Since the M10 cutover the reference bundles are built from the static
// `reference-data` package (committed JSON snapshots) — no network, no
// per-IP rate limit, no Next data cache. These tests pin the new
// contract: the bundles NEVER hit `fetch`, the counts are populated
// from the static snapshots, and the module-level memo is stable and
// resettable.

describe('loadAllReferenceBundles (static-sourced)', () => {
  beforeEach(() => {
    __resetReferenceBundleCacheForTests();
  });

  afterEach(() => {
    vi.unstubAllGlobals();
    __resetReferenceBundleCacheForTests();
  });

  it('builds the bundles without ever calling fetch', async () => {
    const fetchMock = vi.fn(() => {
      throw new Error('fetch must not be called for the static catalog');
    });
    vi.stubGlobal('fetch', fetchMock);

    const bundles = await loadAllReferenceBundles();

    expect(fetchMock).not.toHaveBeenCalled();
    // Populated from the committed snapshots — every category non-empty.
    expect(bundles.counts.vehicle).toBeGreaterThan(0);
    expect(bundles.counts.weapon).toBeGreaterThan(0);
    expect(bundles.counts.item).toBeGreaterThan(0);
    expect(bundles.counts.location).toBeGreaterThan(0);
  });

  it('serves a stable memo across repeated calls', async () => {
    const first = await loadAllReferenceBundles();
    const second = await loadAllReferenceBundles();
    // Same object graph reused (no rebuild).
    expect(second).toBe(first);
    expect(second.catalogs.vehicles).toBe(first.catalogs.vehicles);
  });

  it('rebuilds after the test-reset hook clears the memo', async () => {
    const first = await loadAllReferenceBundles();
    __resetReferenceBundleCacheForTests();
    const rebuilt = await loadAllReferenceBundles();
    // Fresh object graph, identical counts.
    expect(rebuilt).not.toBe(first);
    expect(rebuilt.counts).toEqual(first.counts);
  });

  it('dual-keys the catalog: entries resolve by class_name AND display_name', async () => {
    const { catalogs } = await loadAllReferenceBundles();
    // A vehicle keyed under both its raw class_name and its friendly
    // display_name (the dual-keying <EntityLink> depends on).
    const byClass = catalogs.vehicles.get('aegs_avenger_titan');
    expect(byClass).toBeDefined();
    const byName = catalogs.vehicles.get(
      (byClass?.display_name ?? '').toLowerCase(),
    );
    expect(byName).toBe(byClass);
  });
});
