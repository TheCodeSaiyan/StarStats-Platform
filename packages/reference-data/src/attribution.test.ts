import { describe, expect, it } from 'vitest';

import {
  CIG_ATTRIBUTION,
  DATA_PROVENANCE,
  RSI_CIG,
  RSI_SHIP_MATRIX,
  SHIP_MATRIX_DISCLAIMER,
  SOURCES,
} from './attribution';

describe('attribution (CIG/RSI-only since the M10 cutover)', () => {
  it('ships with rsi-cig provenance — facts + CIG data, no wiki prose', () => {
    expect(DATA_PROVENANCE).toBe('rsi-cig');
  });

  it('credits CIG/RSI only — no Star Citizen Wiki / CC BY-SA source', () => {
    const ids = SOURCES.map((s) => s.id);
    expect(ids).toContain('rsi-cig');
    expect(ids).toContain('rsi-ship-matrix');
    expect(ids).not.toContain('star-citizen-wiki');
    // Every source is a first-party rights holder now.
    expect(SOURCES.every((s) => s.kind === 'first-party')).toBe(true);
    // No source carries a CC BY-SA (or any) licence obligation.
    expect(
      SOURCES.some((s) => JSON.stringify(s).includes('CC BY-SA')),
    ).toBe(false);
  });

  it('marks CIG/RSI as the first-party source for every category', () => {
    expect(RSI_CIG.kind).toBe('first-party');
    expect(RSI_CIG.appliesTo).toEqual(['vehicle', 'weapon', 'item', 'location']);
  });

  it('marks the Ship Matrix as first-party and vehicle-only', () => {
    expect(RSI_SHIP_MATRIX.kind).toBe('first-party');
    expect(RSI_SHIP_MATRIX.appliesTo).toEqual(['vehicle']);
  });

  it('keeps the verbatim CIG Ship Matrix disclaimer used on KB pages', () => {
    // MUST match ShipMatrixDisclaimer.tsx / .test.tsx byte-for-byte.
    expect(SHIP_MATRIX_DISCLAIMER).toBe(
      'Ship specifications, descriptions and images © Cloud Imperium Rights LLC / Cloud Imperium Rights Ltd. StarStats is an unofficial fan site, not endorsed by or affiliated with Cloud Imperium Group.',
    );
  });

  it('surfaces structured CIG-attribution facts for link-woven prose', () => {
    expect(CIG_ATTRIBUTION.sourceName).toBe(
      'Cloud Imperium / Roberts Space Industries',
    );
    expect(CIG_ATTRIBUTION.sourceUrl).toBe('https://robertsspaceindustries.com');
  });
});
