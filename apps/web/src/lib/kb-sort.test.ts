import { describe, expect, it } from 'vitest';
import { parseSortDir, sortKbEntries } from './kb-sort';

interface Row {
  display_name: string;
  maker: string;
}

const rows: Row[] = [
  { display_name: 'Cutlass Black', maker: 'Drake' },
  { display_name: 'Avenger Titan', maker: 'Aegis' },
  { display_name: 'Gladius', maker: 'Aegis' },
];

describe('sortKbEntries', () => {
  it('sorts by primary value ascending by default', () => {
    const out = sortKbEntries(rows, (r) => r.display_name, 'asc');
    expect(out.map((r) => r.display_name)).toEqual([
      'Avenger Titan',
      'Cutlass Black',
      'Gladius',
    ]);
  });

  it('reverses for descending', () => {
    const out = sortKbEntries(rows, (r) => r.display_name, 'desc');
    expect(out.map((r) => r.display_name)).toEqual([
      'Gladius',
      'Cutlass Black',
      'Avenger Titan',
    ]);
  });

  it('breaks ties on display_name so equal primary keys stay stable', () => {
    // Two Aegis entries — must order Avenger Titan before Gladius.
    const out = sortKbEntries(rows, (r) => r.maker, 'asc');
    expect(out.map((r) => r.display_name)).toEqual([
      'Avenger Titan',
      'Gladius',
      'Cutlass Black',
    ]);
  });

  it('does not mutate the input array', () => {
    const copy = [...rows];
    sortKbEntries(rows, (r) => r.display_name, 'desc');
    expect(rows).toEqual(copy);
  });

  it('parseSortDir defaults to asc and only honours desc', () => {
    expect(parseSortDir(undefined)).toBe('asc');
    expect(parseSortDir('')).toBe('asc');
    expect(parseSortDir('nonsense')).toBe('asc');
    expect(parseSortDir('desc')).toBe('desc');
  });
});
