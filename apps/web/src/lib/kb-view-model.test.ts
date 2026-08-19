import { describe, it, expect } from 'vitest';
import { buildVisualModel } from './kb-view-model';
import type { Quantiles } from './kb-stats';

const q = (n = 84): Quantiles => ({ min: 200, p10: 205, p25: 210, p50: 222, p75: 240, p90: 270, max: 275, n });

describe('buildVisualModel', () => {
  it('builds grouped stat rows + radar axes from metadata + bucket', () => {
    const model = buildVisualModel(
      'vehicle',
      { speed: { scm: 262, max: 1425 }, health: 11900 },
      { 'speed.scm': q(), 'speed.max': q(), health: { ...q(), min: 1950, p50: 15550, max: 89300 } },
      'metric',
    );
    const flight = model.groups.find((g) => g.title === 'Flight & handling');
    expect(flight).toBeDefined();
    expect(flight!.rows.some((r) => r.label === 'SCM speed' && r.valueText === '262 m/s')).toBe(true);
    expect(model.radarAxes.length).toBeGreaterThanOrEqual(0);
  });

  it('drops groups with no present fields', () => {
    const model = buildVisualModel('vehicle', {}, {}, 'metric');
    expect(model.groups).toEqual([]);
  });

  it('builds a weapon radar + DPS headline from combat metadata', () => {
    const meta = {
      personal_weapon: {
        damage: { dps_total: 250, alpha_total: 60 },
        rof: 600,
        effective_range: 120,
        ammunition: { speed: 800 },
      },
    };
    const bucket: Record<string, Quantiles> = {
      'personal_weapon.damage.dps_total': q(),
      'personal_weapon.damage.alpha_total': q(),
      'personal_weapon.rof': q(),
      'personal_weapon.effective_range': q(),
      'personal_weapon.ammunition.speed': q(),
    };
    const model = buildVisualModel('weapon', meta, bucket, 'metric');
    expect(model.radarAxes.length).toBeGreaterThanOrEqual(3);
    expect(model.radarAxes.map((a) => a.label)).toContain('DPS');
    expect(model.headline.some((r) => r.label === 'DPS')).toBe(true);
    const dmg = model.groups.find((g) => g.title === 'Damage & fire');
    expect(dmg?.rows.some((r) => r.label === 'DPS')).toBe(true);
  });

  it('renders NO radar for items (heterogeneous) but keeps headline + bars', () => {
    const meta = { mass: 12, dimension: { volume_converted: 3000 }, durability: { health: 5000 } };
    const bucket: Record<string, Quantiles> = {
      mass: q(),
      'dimension.volume_converted': q(),
      'durability.health': q(),
    };
    const model = buildVisualModel('item', meta, bucket, 'metric');
    expect(model.radarAxes).toEqual([]);
    expect(model.headline.some((r) => r.label === 'Armor HP')).toBe(true);
  });

  it('renders NO radar for locations; headline carries scale metrics', () => {
    const meta = { size: 4000, child_count: 12, mission_count: 80 };
    const bucket: Record<string, Quantiles> = {
      size: q(),
      child_count: q(),
      mission_count: q(),
    };
    const model = buildVisualModel('location', meta, bucket, 'metric');
    expect(model.radarAxes).toEqual([]);
    expect(model.headline.map((r) => r.label)).toEqual(
      expect.arrayContaining(['Sub-locations', 'Missions', 'Diameter']),
    );
  });
});
