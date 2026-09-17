import { describe, it, expect } from 'vitest';
import { prettyHangarItem, rsiStoreSearchUrl } from './hangar-label';

describe('prettyHangarItem', () => {
  it('strips the "Standalone Ships - " prefix and maps to vehicle', () => {
    expect(prettyHangarItem('Standalone Ships - Railen')).toEqual({
      label: 'Railen',
      category: 'vehicle',
    });
  });

  it('drops the "Paints - <ship> - " prefix, keeps the paint name, no link', () => {
    expect(prettyHangarItem('Paints - Railen - Uamchuai Paint')).toEqual({
      label: 'Uamchuai Paint',
      category: null,
    });
  });

  it('maps a weapon kind to the weapon category', () => {
    expect(prettyHangarItem('Weapons - Behring P4-AR', 'weapon')).toEqual({
      label: 'Behring P4-AR',
      category: 'weapon',
    });
  });

  it('treats a Subscribers Store flair item as cosmetic (no link)', () => {
    expect(
      prettyHangarItem('Subscribers Store - Salvaged Skull Relax to the Max Set'),
    ).toEqual({
      label: 'Salvaged Skull Relax to the Max Set',
      category: null,
    });
  });

  it('prefers an explicit ship/vehicle kind over the prefix', () => {
    expect(prettyHangarItem('Standalone Ships - Cutlass Black', 'ground vehicle')).toEqual(
      { label: 'Cutlass Black', category: 'vehicle' },
    );
  });

  it('treats skin/upgrade/add-on kinds as cosmetic (no link)', () => {
    expect(prettyHangarItem('Upgrades - 300i to 325a', 'upgrade').category).toBeNull();
    expect(prettyHangarItem('Add-Ons - Hangar Flair', 'add-on').category).toBeNull();
    expect(prettyHangarItem('Some Skin', 'skin').category).toBeNull();
  });

  it('infers vehicle from a "Ships - " prefix when kind is absent', () => {
    expect(prettyHangarItem('Ships - Aegis Gladius')).toEqual({
      label: 'Aegis Gladius',
      category: 'vehicle',
    });
  });

  it('falls back to a weapon heuristic on the name when nothing else signals it', () => {
    expect(prettyHangarItem('Behring P4-AR Rifle').category).toBe('weapon');
  });

  it('leaves an unprefixed, unknown item as plain text with no link', () => {
    expect(prettyHangarItem('Mystery Box')).toEqual({
      label: 'Mystery Box',
      category: null,
    });
  });

  it('is robust to extra whitespace and empty input', () => {
    expect(prettyHangarItem('   Standalone Ships  -  Railen  ')).toEqual({
      label: 'Railen',
      category: 'vehicle',
    });
    expect(prettyHangarItem('')).toEqual({ label: '', category: null });
  });
});

describe('rsiStoreSearchUrl', () => {
  it('builds the confirmed live shape', () => {
    // Verified against a working store URL supplied by the account
    // holder, 2026-09-17. The parameter is `keywords`, NOT `q` — `q`
    // is accepted and silently ignored, so getting this wrong yields
    // a link that looks like a search and shows an unfiltered
    // catalogue.
    expect(rsiStoreSearchUrl('emoto')).toBe(
      'https://robertsspaceindustries.com/en/store/pledge/browse/?page=1&keywords=emoto',
    );
  });

  it('encodes names with spaces and punctuation', () => {
    // Hangar names are full pledge titles, not slugs.
    const url = rsiStoreSearchUrl('Aegis Avenger Titan');
    expect(url).toContain('keywords=Aegis+Avenger+Titan');
    expect(url).not.toContain(' ');
  });

  it('encodes a name that would otherwise break the query string', () => {
    const url = rsiStoreSearchUrl('Paints - Constellation & Polar');
    // `&` must not start a new parameter.
    expect(url).not.toMatch(/&Polar/);
    expect(url).toContain('%26');
  });

  it('returns null for an empty or whitespace name', () => {
    // A blank keyword would link to the unfiltered catalogue, which
    // is worse than rendering no link at all.
    expect(rsiStoreSearchUrl('')).toBeNull();
    expect(rsiStoreSearchUrl('   ')).toBeNull();
  });

  it('targets the browse root, not a category', () => {
    // Category paths exist and work, but choosing one per item means
    // classifying it; a wrong category shows no results. Subscriber
    // items make that unreliable — that category's contents differ
    // per account.
    expect(rsiStoreSearchUrl('x')).toContain('/store/pledge/browse/?');
    expect(rsiStoreSearchUrl('x')).not.toContain('/browse/extras/');
  });
});
