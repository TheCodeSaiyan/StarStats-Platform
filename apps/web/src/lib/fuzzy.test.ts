import { describe, it, expect } from 'vitest';
import { editDistance, fuzzyScore, normalizeForMatch, rankFuzzy } from './fuzzy';

const ships = [
  'Avenger Stalker',
  'Avenger Titan',
  'Gladius',
  'Arrow',
  'C1 Spirit',
  "Rod's Fuel 'N Supplies",
  'Stalwart',
];

describe('fuzzy', () => {
  it('normalises punctuation and case away', () => {
    expect(normalizeForMatch("Rod's Fuel 'N Supplies")).toBe('rod s fuel n supplies');
    expect(normalizeForMatch('  C1  Spirit ')).toBe('c1 spirit');
  });

  it('counts a transposition as one edit', () => {
    expect(editDistance('gladuis', 'gladius')).toBe(1);
    expect(editDistance('stalkr', 'stalker')).toBe(1);
    expect(editDistance('arrow', 'arrow')).toBe(0);
  });

  it('matches a word prefix, a substring, a typo and a subsequence', () => {
    expect(fuzzyScore('glad', 'Gladius')).not.toBeNull();
    expect(fuzzyScore('enger', 'Avenger Stalker')).not.toBeNull();
    expect(fuzzyScore('gladuis', 'Gladius')).not.toBeNull();
    expect(fuzzyScore('avstk', 'Avenger Stalker')).not.toBeNull();
  });

  it('rejects a query with a token that lands nowhere', () => {
    expect(fuzzyScore('avenger xyz', 'Avenger Stalker')).toBeNull();
    expect(fuzzyScore('', 'Gladius')).toBeNull();
  });

  it('ranks the obvious hit first', () => {
    expect(rankFuzzy('glad', ships, (s) => s)[0]).toBe('Gladius');
    expect(rankFuzzy('gladuis', ships, (s) => s)[0]).toBe('Gladius');
    expect(rankFuzzy('avstk', ships, (s) => s)[0]).toBe('Avenger Stalker');
    expect(rankFuzzy('stalker', ships, (s) => s)[0]).toBe('Avenger Stalker');
    expect(rankFuzzy('c1', ships, (s) => s)[0]).toBe('C1 Spirit');
    expect(rankFuzzy('rods fuel', ships, (s) => s)[0]).toBe("Rod's Fuel 'N Supplies");
  });

  it('prefers a contiguous phrase over scattered tokens, and shorter labels on ties', () => {
    const r = rankFuzzy('avenger', ships, (s) => s);
    expect(r.slice(0, 2)).toEqual(['Avenger Titan', 'Avenger Stalker']);
    expect(rankFuzzy('a', ships, (s) => s, 2)).toHaveLength(2);
  });
});
