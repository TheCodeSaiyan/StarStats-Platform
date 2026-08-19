import { describe, it, expect } from 'vitest';
import {
  prettyLocationLabel,
  aggregateLocationBuckets,
} from '@/lib/class-name-parts';

// Locks the "not resolving" fix: the exact raw values seen on the live
// dashboard (routes / places-visited) now render as readable labels.
describe('prettyLocationLabel', () => {
  it.each([
    ['LOC_RR_S1_L1', 'HUR-L1 Green Glade Station'],
    ['LOC_RR_S1_L2', 'HUR-L2 Faithful Dream Station'],
    ['LOC_RR_S1_L5', 'HUR-L5 High Course Station'],
    ['LOC_RR_S2_L1', 'CRU-L1 Ambitious Dream Station'],
    ['LOC_RR_S3_L1', 'ARC-L1 Wide Forest Station'],
    ['LOC_RR_S3_L4', 'ARC-L4 Faint Glen Station'],
    ['LOC_RR_S4_L1', 'MIC-L1 Shallow Frontier Station'],
    ['rs_ext_cru-leo1', 'Seraphim Station'],
  ])('resolves station engine id %s', (raw, expected) => {
    expect(prettyLocationLabel(raw)).toBe(expected);
  });

  it('keeps the route when labelling a jump point', () => {
    expect(prettyLocationLabel('LOC_rs_ext_stan-magnus_jp1')).toBe(
      'Stanton ↔ Magnus · Jump Point 1',
    );
    expect(prettyLocationLabel('LOC_rs_ext_stan-pyro_jp1')).toBe(
      'Stanton ↔ Pyro · Jump Point 1',
    );
  });

  it('keeps the object type when labelling a comm array', () => {
    expect(prettyLocationLabel('OOC_Stanton2c_CommArray')).toBe(
      'Stanton Comm Array 2c',
    );
  });

  it('normalizes the legacy Ariel spelling to Arial', () => {
    expect(prettyLocationLabel('OOC_Stanton_1a_Ariel')).toBe('Arial');
  });

  it.each([
    ['ObjectContainer_RestStop', 'Rest Stop'],
    ['ab_mine_stanton2_med_005', 'Asteroid mining node'],
    ['ab_collector_gas_Stanton2', 'Gas collection node'],
    [
      'arccorp_cluster_001_frost_{C4F29ABD-9E68-4FD7-9A70-A0522884BF50}.socpak',
      'Asteroid cluster',
    ],
    ['racing_static_st2c_ghexasteroid', 'Race track'],
  ])('groups procedural destination %s', (raw, expected) => {
    expect(prettyLocationLabel(raw)).toBe(expected);
  });

  it('collapses per-mission quantum beacons to a generic label', () => {
    expect(prettyLocationLabel('MISSION_QT_Quantum_Beacon_718368901207')).toBe(
      'Mission beacon',
    );
    expect(
      prettyLocationLabel('MISSION_QT_Quantum_Beacon_ShortRange_Salvage_720552809368'),
    ).toBe('Mission beacon · salvage');
  });

  it('resolves place engine ids', () => {
    expect(prettyLocationLabel('NewBabbage_LOC')).toBe('New Babbage');
  });

  it('takes the most-specific segment of a pipe hierarchy', () => {
    expect(prettyLocationLabel('Stanton|Microtech|')).toBe('microTech');
    expect(prettyLocationLabel('Stanton|microTech|New Babbage')).toBe('New Babbage');
  });

  it('handles empty / degenerate values', () => {
    expect(prettyLocationLabel('||')).toBe('Unknown');
    expect(prettyLocationLabel('')).toBe('Unknown');
  });
});

describe('aggregateLocationBuckets', () => {
  it('merges buckets that collapse to the same label, summing counts', () => {
    const agg = aggregateLocationBuckets([
      { value: 'MISSION_QT_Quantum_Beacon_718368901207', count: 2 },
      { value: 'MISSION_QT_Quantum_Beacon_718384911828', count: 3 },
      { value: 'LOC_RR_S1_L1', count: 4 },
    ]);
    // Sorted by count desc: merged beacons (5) lead, then the rest stop (4).
    expect(agg[0]).toEqual(
      expect.objectContaining({ label: 'Mission beacon', count: 5 }),
    );
    expect(agg[0].raws).toHaveLength(2); // two beacons merged
    expect(
      agg.find((a) => a.label === 'HUR-L1 Green Glade Station')?.count,
    ).toBe(4);
  });

  it('drops dynamic party and nav markers from route destinations', () => {
    const agg = aggregateLocationBuckets([
      { value: 'PartyMemberMarker_272192089137', count: 18 },
      { value: 'PartyMemberMarker_200131831145', count: 17 },
      { value: 'NavPoint_Dynamic_348490534863', count: 3 },
      { value: 'Area18_City_objectContainer', count: 75 },
    ]);

    expect(agg).toEqual([
      expect.objectContaining({ label: 'Area18', count: 75 }),
    ]);
  });

  it('merges procedural destinations without leaking runtime ids', () => {
    const agg = aggregateLocationBuckets([
      { value: 'ab_mine_stanton2_med_005', count: 12 },
      { value: 'ab_mine_stanton2_med_010', count: 12 },
      {
        value:
          'arccorp_cluster_001_frost_{C4F29ABD-9E68-4FD7-9A70-A0522884BF50}.socpak',
        count: 8,
      },
      {
        value:
          'shubin_cluster_001_sand_{B8AA34E0-BBDC-4000-A3AE-3705210EE06E}.socpak',
        count: 4,
      },
    ]);

    expect(agg).toEqual([
      expect.objectContaining({ label: 'Asteroid mining node', count: 24 }),
      expect.objectContaining({ label: 'Asteroid cluster', count: 12 }),
    ]);
  });
});
