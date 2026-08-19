import { describe, it, expect } from 'vitest';
import { buildDetailGroups } from './kb-detail';

describe('buildDetailGroups — weapon & item', () => {
  it('surfaces weapon combat stats in a Damage & fire group', () => {
    const groups = buildDetailGroups('weapon', {
      personal_weapon: {
        damage: { dps_total: 250, alpha_total: 60 },
        rof: 600,
        effective_range: 120,
        ammunition: { speed: 800 },
      },
      mass: 4.2,
    });
    const byTitle = Object.fromEntries(groups.map((g) => [g.title, g.rows]));
    expect(byTitle['Damage & fire']).toContainEqual({ label: 'DPS', value: '250 dps' });
    expect(byTitle['Range & handling']).toContainEqual({ label: 'Effective range', value: '120 m' });
  });

  it('drops the item Protection group for non-armor items, renders it for armor', () => {
    const plain = buildDetailGroups('item', { mass: 2, size: 1 });
    expect(plain.some((g) => g.title === 'Protection')).toBe(false);

    const armor = buildDetailGroups('item', {
      mass: 12,
      durability: { health: 5000 },
      armor: { deflection: { physical: 200 } },
    });
    const prot = armor.find((g) => g.title === 'Protection');
    expect(prot?.rows).toContainEqual({ label: 'Health', value: '5,000 hp' });
  });
});

describe('buildDetailGroups', () => {
  it('groups vehicle metadata into themed sections with units, dropping zeros', () => {
    const groups = buildDetailGroups('vehicle', {
      speed: { scm: 215, max: 1210, boost_forward: 0 }, // boost 0 → dropped
      agility: { pitch: 60, yaw: 55, roll: 120, acceleration: 50 },
      quantum: { quantum_speed: 200000000, quantum_range: 12 },
      health: 11900,
      shield_hp: 4488,
      weaponry: { fixed_weapons: { dps_total: 1234.5 } },
      turrets: { manned: [], remote: [{ x: 1 }] }, // manned empty → dropped, remote count 1
      ports: [{}, {}, {}],
      msrp: 60,
      production_status: { en_EN: 'flight-ready', de_DE: 'x' },
    });

    const byTitle = Object.fromEntries(groups.map((g) => [g.title, g.rows]));

    const flight = byTitle['Flight & handling'];
    expect(flight).toBeDefined();
    expect(flight).toContainEqual({ label: 'SCM speed', value: '215 m/s' });
    expect(flight).toContainEqual({ label: 'Max speed', value: '1,210 m/s' });
    // boost_forward was 0 → not present
    expect(flight!.some((r) => r.label === 'Boost (fwd)')).toBe(false);

    const survive = byTitle['Survivability'];
    expect(survive).toContainEqual({ label: 'Hull HP', value: '11,900 hp' });

    const weapons = byTitle['Weaponry'];
    // Values >= 100 round to whole numbers for display.
    expect(weapons).toContainEqual({ label: 'Pilot DPS', value: '1,235 dps' });
    expect(weapons).toContainEqual({ label: 'Remote turrets', value: '1' });
    expect(weapons).toContainEqual({ label: 'Hardpoints', value: '3' });
    // manned turrets array is empty → dropped
    expect(weapons!.some((r) => r.label === 'Manned turrets')).toBe(false);

    const acq = byTitle['Acquisition'];
    expect(acq).toContainEqual({ label: 'Pledge price', value: '$60' });
    expect(acq).toContainEqual({ label: 'Production status', value: 'flight-ready' });
  });

  it('drops groups with no present fields', () => {
    // Only flight data present → only that group renders.
    const groups = buildDetailGroups('vehicle', { speed: { scm: 100 } });
    expect(groups.map((g) => g.title)).toEqual(['Flight & handling']);
  });

  it('returns no groups for empty metadata', () => {
    expect(buildDetailGroups('vehicle', {})).toEqual([]);
    expect(buildDetailGroups('item', {})).toEqual([]);
  });

  it('builds location overview/profile with text + bool + count', () => {
    const groups = buildDetailGroups('location', {
      system: 'Stanton System',
      designation: 'Stanton Ib',
      type: { name: 'Moon', classification: 'Planetary satellite' },
      parent: { name: 'Hurston' },
      child_count: 37,
      has_resources: true,
      is_scannable: false,
      amenities: [],
    });
    const byTitle = Object.fromEntries(groups.map((g) => [g.title, g.rows]));
    expect(byTitle['Overview']).toContainEqual({ label: 'System', value: 'Stanton System' });
    expect(byTitle['Overview']).toContainEqual({ label: 'Type', value: 'Moon' });
    expect(byTitle['Overview']).toContainEqual({ label: 'Orbits', value: 'Hurston' });
    expect(byTitle['Profile']).toContainEqual({ label: 'Sub-locations', value: '37' });
    expect(byTitle['Profile']).toContainEqual({ label: 'Has resources', value: 'Yes' });
    expect(byTitle['Profile']).toContainEqual({ label: 'Scannable', value: 'No' });
    // empty amenities array → dropped
    expect(byTitle['Profile'].some((r) => r.label === 'Amenities')).toBe(false);
  });
});
