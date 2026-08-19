import { expect, test } from 'vitest';

import {
  groupForItem,
  isExcludedPort,
  isLoadoutBurstPayload,
  pickFullestLoadoutBurst,
  slotForClassification,
} from '@/lib/loadout';

test('excludes anatomy + radar + mobiglas, keeps gear', () => {
  for (const p of [
    'Eyes_ItemPort',
    'Hair_ItemPort',
    'Body_ItemPort',
    'radar',
    'mobiglas_attach',
    'universal_necksock',
    'Lens_ItemPort',
  ])
    expect(isExcludedPort(p)).toBe(true);
  for (const p of [
    'Armor_Helmet',
    'wep_sidearm',
    'magazine_attach_1',
    'helmet_visor',
    'Eye_Accessories_ItemPort',
    'grenade_attach_1',
  ])
    expect(isExcludedPort(p)).toBe(false);
});

test('classification → slot', () => {
  expect(slotForClassification('FPS.Armor.Helmet')).toBe('head');
  expect(slotForClassification('FPS.Armor.Backpack')).toBe('back');
  expect(slotForClassification('FPS.Weapon.Small')).toBeNull();
  expect(slotForClassification(undefined)).toBeNull();
});

test('item → carried group', () => {
  expect(groupForItem('FPS.Weapon.Small', 'wep_sidearm')).toBe('weapons');
  expect(groupForItem('FPS.WeaponAttachment.Magazine', 'magazine_attach_1')).toBe('magazines');
  expect(groupForItem('FPS.Consumable.Medical', 'medPen_attach_1')).toBe('consumables');
  expect(groupForItem(undefined, 'grenade_attach_1')).toBe('throwables');
  expect(groupForItem(undefined, 'utility_attach_1', 'item')).toBe('utility');
});

test('isLoadoutBurstPayload guards kind + items', () => {
  expect(isLoadoutBurstPayload({ kind: 'loadout_restore', items: [] })).toBe(true);
  expect(isLoadoutBurstPayload({ kind: 'other', items: [] })).toBe(false);
  expect(isLoadoutBurstPayload({ kind: 'loadout_restore' })).toBe(false);
  expect(isLoadoutBurstPayload(null)).toBe(false);
  expect(isLoadoutBurstPayload('x')).toBe(false);
});

test('pickFullestLoadoutBurst picks the burst with the most items, not the newest', () => {
  const events = [
    // newest first — a partial re-equip (2 items)
    {
      event_type: 'burst_summary',
      payload: {
        kind: 'loadout_restore',
        items: [
          { class: 'a', port: 'p', category: 'item' },
          { class: 'b', port: 'p', category: 'item' },
        ],
      },
    },
    // an unrelated burst
    { event_type: 'burst_summary', payload: { kind: 'terrain_load', items: [] } },
    // older — the full spawn (3 items) — should win
    {
      event_type: 'burst_summary',
      payload: {
        kind: 'loadout_restore',
        items: [
          { class: 'a', port: 'p', category: 'item' },
          { class: 'b', port: 'p', category: 'item' },
          { class: 'c', port: 'p', category: 'item' },
        ],
      },
    },
  ];
  const chosen = pickFullestLoadoutBurst(events);
  expect(chosen?.payload).toMatchObject({ kind: 'loadout_restore' });
  expect((chosen?.payload as { items: unknown[] }).items).toHaveLength(3);
});

test('pickFullestLoadoutBurst returns undefined when no loadout burst exists', () => {
  expect(
    pickFullestLoadoutBurst([
      { event_type: 'burst_summary', payload: { kind: 'terrain_load', items: [] } },
      { event_type: 'session_start', payload: {} },
    ]),
  ).toBeUndefined();
});
