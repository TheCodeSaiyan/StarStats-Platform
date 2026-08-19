import { describe, expect, it } from 'vitest';

import {
  parseShipMatrix,
  shipMatrixForCategory,
  shipMatrixMediaUrls,
  shipMatrixSpecRows,
} from './ship-matrix';

// A representative `metadata.ship_matrix` blob matching the spec shape.
const SAMPLE = {
  specs: {
    length: 23.5,
    beam: 21.5,
    height: 6.5,
    mass: 226345,
    scm_speed: 215,
    afterburner_speed: 1275,
    min_crew: 1,
    max_crew: 2,
    cargo: 46,
  },
  production_status: 'flight-ready',
  description: 'A versatile medium freighter.',
  media: ['https://media.example/1.jpg', 'https://media.example/2.jpg'],
  matched_by: 'name',
  matched_at: '2026-06-12T00:00:00Z',
};

describe('parseShipMatrix', () => {
  it('parses a well-formed blob', () => {
    const parsed = parseShipMatrix(SAMPLE);
    expect(parsed).not.toBeNull();
    expect(parsed?.description).toBe('A versatile medium freighter.');
    expect(parsed?.production_status).toBe('flight-ready');
    expect(parsed?.media).toEqual([
      'https://media.example/1.jpg',
      'https://media.example/2.jpg',
    ]);
    expect(parsed?.specs.length).toBe(23.5);
    expect(parsed?.specs.max_crew).toBe(2);
  });

  it('returns null for absent / non-object input', () => {
    expect(parseShipMatrix(undefined)).toBeNull();
    expect(parseShipMatrix(null)).toBeNull();
    expect(parseShipMatrix('flight-ready')).toBeNull();
    expect(parseShipMatrix(42)).toBeNull();
    expect(parseShipMatrix([])).toBeNull();
  });

  it('tolerates a blob with no specs object', () => {
    const parsed = parseShipMatrix({ description: 'no specs here' });
    expect(parsed).not.toBeNull();
    expect(parsed?.description).toBe('no specs here');
    // Missing specs object collapses to an empty specs record.
    expect(parsed?.specs).toEqual({});
  });

  it('drops non-numeric spec fields rather than coercing them', () => {
    const parsed = parseShipMatrix({
      specs: { length: 23.5, cargo: 'lots', mass: null },
    });
    expect(parsed?.specs.length).toBe(23.5);
    expect(parsed?.specs.cargo).toBeUndefined();
    expect(parsed?.specs.mass).toBeUndefined();
  });

  it('filters media down to non-empty strings', () => {
    const parsed = parseShipMatrix({
      media: ['https://ok', '', 7, null, 'https://ok2'],
    });
    expect(parsed?.media).toEqual(['https://ok', 'https://ok2']);
  });

  it('defaults media to an empty array when absent', () => {
    const parsed = parseShipMatrix({ description: 'x' });
    expect(parsed?.media).toEqual([]);
  });

  it('ignores non-string description / production_status', () => {
    const parsed = parseShipMatrix({ description: 12, production_status: {} });
    expect(parsed?.description).toBeUndefined();
    expect(parsed?.production_status).toBeUndefined();
  });
});

describe('shipMatrixSpecRows', () => {
  it('emits labelled rows only for present numeric fields, in a stable order', () => {
    const parsed = parseShipMatrix(SAMPLE);
    const rows = shipMatrixSpecRows(parsed!);
    const labels = rows.map((r) => r.label);
    // Dimensions come before speeds before crew before cargo.
    expect(labels[0]).toBe('Length');
    expect(labels).toContain('SCM speed');
    expect(labels).toContain('Crew');
    expect(labels).toContain('Cargo');
    // Production status is included as a (non-numeric) row at the end.
    expect(labels).toContain('Production status');
  });

  it('omits rows for missing fields', () => {
    const parsed = parseShipMatrix({ specs: { length: 10 } });
    const rows = shipMatrixSpecRows(parsed!);
    const labels = rows.map((r) => r.label);
    expect(labels).toEqual(['Length']);
  });

  it('collapses min/max crew into a single range row', () => {
    const parsed = parseShipMatrix({ specs: { min_crew: 1, max_crew: 4 } });
    const rows = shipMatrixSpecRows(parsed!);
    const crew = rows.find((r) => r.label === 'Crew');
    expect(crew?.value).toBe('1–4');
  });

  it('shows a single crew value when min equals max', () => {
    const parsed = parseShipMatrix({ specs: { min_crew: 2, max_crew: 2 } });
    const rows = shipMatrixSpecRows(parsed!);
    const crew = rows.find((r) => r.label === 'Crew');
    expect(crew?.value).toBe('2');
  });

  it('returns no crew row when both crew fields are absent', () => {
    const parsed = parseShipMatrix({ specs: { length: 10 } });
    const rows = shipMatrixSpecRows(parsed!);
    expect(rows.find((r) => r.label === 'Crew')).toBeUndefined();
  });
});

describe('shipMatrixForCategory', () => {
  const metadata = { ship_matrix: SAMPLE } as Record<string, unknown>;

  it('returns the parsed blob for the vehicle category', () => {
    const sm = shipMatrixForCategory('vehicle', metadata);
    expect(sm).not.toBeNull();
    expect(sm?.description).toBe('A versatile medium freighter.');
  });

  it('returns null for non-vehicle categories even with a ship_matrix blob', () => {
    expect(shipMatrixForCategory('weapon', metadata)).toBeNull();
    expect(shipMatrixForCategory('item', metadata)).toBeNull();
    expect(shipMatrixForCategory('location', metadata)).toBeNull();
  });

  it('returns null for a vehicle with no ship_matrix in metadata', () => {
    expect(shipMatrixForCategory('vehicle', { other: 1 })).toBeNull();
  });
});

describe('shipMatrixMediaUrls', () => {
  it('builds one same-origin relative proxy URL per media entry', () => {
    const parsed = parseShipMatrix(SAMPLE)!;
    const urls = shipMatrixMediaUrls(parsed, 'AEGS_Avenger');
    expect(urls).toEqual([
      '/kb/media/vehicle/AEGS_Avenger/0',
      '/kb/media/vehicle/AEGS_Avenger/1',
    ]);
  });

  it('returns an empty array when media is absent', () => {
    const parsed = parseShipMatrix({ description: 'x' })!;
    expect(shipMatrixMediaUrls(parsed, 'AEGS_Avenger')).toEqual([]);
  });

  it('url-encodes the class name and never embeds an absolute API host', () => {
    const parsed = parseShipMatrix({ media: ['https://x'] })!;
    const urls = shipMatrixMediaUrls(parsed, 'A B/C');
    expect(urls[0]).toBe('/kb/media/vehicle/A%20B%2FC/0');
    // Critically: no internal/absolute host leaks into the browser URL
    // (this was the bug — apiBase() was the internal compose hostname).
    expect(urls[0].startsWith('/')).toBe(true);
    expect(urls[0]).not.toContain('http');
  });
});
