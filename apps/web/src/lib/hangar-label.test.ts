import { describe, it, expect } from 'vitest';
import { prettyHangarItem } from './hangar-label';

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
