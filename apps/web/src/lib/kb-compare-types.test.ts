import { describe, it, expect } from 'vitest';
import {
  buildComparisonMatrix,
  buildComparisonRadar,
  buildLeaderboard,
  type CompareEntry,
} from './kb-compare-types';

const ship = (slug: string, name: string, m: Record<string, number>): CompareEntry => ({
  slug, display_name: name, class_name: slug.toUpperCase(), peer_group: 'combat', metrics: m,
});
const avenger = ship('avenger', 'Avenger', { 'speed.scm': 262, health: 11900, 'weaponry.fixed_weapons.dps_total': 2360, mass: 49000 });
const gladius = ship('gladius', 'Gladius', { 'speed.scm': 226, health: 6110, 'weaponry.fixed_weapons.dps_total': 1598, mass: 41000 });
const sabre = ship('sabre', 'Sabre', { 'speed.scm': 223, health: 25013, 'weaponry.fixed_weapons.dps_total': 2182, mass: 52000 });

describe('buildComparisonMatrix', () => {
  it('pins anchor first, sorts others by the chosen metric, marks leader, normalises bars', () => {
    const m = buildComparisonMatrix('vehicle', avenger, [gladius, sabre], 'metric', { key: 'speed.scm', dir: 'desc' });
    expect(m.columns[0].slug).toBe('avenger');
    expect(m.columns.slice(1).map((c) => c.slug)).toEqual(['gladius', 'sabre']);
    const hull = m.rows.find((r) => r.key === 'health')!;
    const sabreIdx = m.columns.findIndex((c) => c.slug === 'sabre');
    expect(hull.cells[sabreIdx].isLeader).toBe(true);
    expect(hull.cells[0].isLeader).toBe(false);
    expect(hull.cells[0].fillPct).toBeGreaterThanOrEqual(0);
  });

  it('formats imperial units', () => {
    const m = buildComparisonMatrix('vehicle', avenger, [gladius], 'imperial', { key: 'speed.scm', dir: 'desc' });
    const scm = m.rows.find((r) => r.key === 'speed.scm')!;
    expect(scm.cells[0].text).toContain('ft/s');
  });
});

describe('buildLeaderboard per category', () => {
  const entry = (slug: string, m: Record<string, number>): CompareEntry => ({
    slug, display_name: slug, class_name: slug.toUpperCase(), peer_group: 'x', metrics: m,
  });

  it('weapon superlatives pick the DPS / range leaders', () => {
    const a = entry('p4ar', { 'personal_weapon.damage.dps_total': 250, 'personal_weapon.effective_range': 120 });
    const b = entry('arrowhead', { 'personal_weapon.damage.dps_total': 180, 'personal_weapon.effective_range': 900 });
    const board = buildLeaderboard('weapon', [a, b], 'metric');
    expect(board.find((c) => c.label === 'Highest DPS')?.winnerName).toBe('p4ar');
    expect(board.find((c) => c.label === 'Longest range')?.winnerName).toBe('arrowhead');
  });

  it('location superlatives pick the busiest / largest', () => {
    const a = entry('lorville', { mission_count: 600, size: 4000 });
    const b = entry('area18', { mission_count: 400, size: 9000 });
    const board = buildLeaderboard('location', [a, b], 'metric');
    expect(board.find((c) => c.label === 'Most missions')?.winnerName).toBe('lorville');
    expect(board.find((c) => c.label === 'Largest')?.winnerName).toBe('area18');
  });
});

describe('buildComparisonRadar', () => {
  it('produces one series per ship with per-axis fractions scaled to the set', () => {
    const r = buildComparisonRadar([avenger, gladius, sabre], ['speed.scm', 'health']);
    expect(r.axes).toEqual(['speed.scm', 'health']);
    expect(r.series.map((s) => s.slug)).toEqual(['avenger', 'gladius', 'sabre']);
    const av = r.series.find((s) => s.slug === 'avenger')!;
    expect(av.values[0]).toBeCloseTo(1, 5);
  });
});

describe('buildLeaderboard', () => {
  it('names the winner for each vehicle superlative', () => {
    const lb = buildLeaderboard('vehicle', [avenger, gladius, sabre], 'metric');
    const fastest = lb.find((c) => c.key === 'speed.scm')!;
    expect(fastest.winnerName).toBe('Avenger');
    const toughest = lb.find((c) => c.key === 'health')!;
    expect(toughest.winnerName).toBe('Sabre');
  });
});

describe('buildComparisonMatrix edge cases', () => {
  it('computes exact fillPct normalisation across the row', () => {
    // health across [avenger 11900, gladius 6110, sabre 25013] → min 6110, max 25013
    const m = buildComparisonMatrix('vehicle', avenger, [gladius, sabre], 'metric', { key: 'speed.scm', dir: 'desc' });
    const hull = m.rows.find((r) => r.key === 'health')!;
    const avIdx = m.columns.findIndex((c) => c.slug === 'avenger');
    const expected = ((11900 - 6110) / (25013 - 6110)) * 100;
    expect(hull.cells[avIdx].fillPct).toBeCloseTo(expected, 4);
  });

  it('uses 100% fill when all values in a row are equal', () => {
    const a = ship('a', 'A', { 'speed.scm': 200 });
    const b = ship('b', 'B', { 'speed.scm': 200 });
    const m = buildComparisonMatrix('vehicle', a, [b], 'metric', { key: 'speed.scm', dir: 'desc' });
    const scm = m.rows.find((r) => r.key === 'speed.scm')!;
    expect(scm.cells.every((c) => c.fillPct === 100)).toBe(true);
    // equal values → no single leader
    expect(scm.cells.every((c) => c.isLeader === true)).toBe(true); // both equal the max
  });

  it('renders an absent metric as a context-free cell', () => {
    const a = ship('a', 'A', { 'speed.scm': 200 });          // has scm, no health
    const b = ship('b', 'B', { 'speed.scm': 210, health: 5000 });
    const m = buildComparisonMatrix('vehicle', a, [b], 'metric', { key: 'speed.scm', dir: 'desc' });
    const hull = m.rows.find((r) => r.key === 'health')!;
    const aIdx = m.columns.findIndex((c) => c.slug === 'a');
    expect(hull.cells[aIdx]).toMatchObject({ value: null, text: '—', fillPct: null, isLeader: false });
  });
});

describe('buildComparisonRadar edge cases', () => {
  it('floors a missing axis value at 0.06 and centres a uniform axis at 0.5', () => {
    const a = ship('a', 'A', { 'speed.scm': 200 });          // missing health
    const b = ship('b', 'B', { 'speed.scm': 200, health: 1000 }); // equal scm → uniform axis
    const r = buildComparisonRadar([a, b], ['speed.scm', 'health']);
    // speed.scm equal across set → 0.5 for both
    expect(r.series.map((s) => s.values[0])).toEqual([0.5, 0.5]);
    // ship a missing health → floored 0.06
    expect(r.series.find((s) => s.slug === 'a')!.values[1]).toBeCloseTo(0.06, 5);
  });
});

describe('buildLeaderboard edge cases', () => {
  it('drops a superlative whose metric is absent across the whole set', () => {
    // none of these carry shield_hp → no "Strongest shield" card
    const lb = buildLeaderboard('vehicle', [avenger, gladius, sabre], 'metric');
    expect(lb.some((c) => c.key === 'shield_hp')).toBe(false);
    // and a present one IS there
    expect(lb.some((c) => c.key === 'speed.scm')).toBe(true);
  });
});
