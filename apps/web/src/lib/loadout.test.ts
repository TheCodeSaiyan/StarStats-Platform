import { expect, test } from 'vitest';

import {
  groupForItem,
  isCompleteRestore,
  isExcludedPort,
  isLoadoutBurstPayload,
  listLoadoutBursts,
  pickLoadoutBurst,
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


// ── Snapshot selection ─────────────────────────────────────────────────
//
// Fixtures mirror the shape that produced the live bug: an OLD kit with
// more items than a NEWER one. Under the previous "most items wins" rule
// the old kit was shown indefinitely.

const item = (cls: string, port = 'p') => ({ class: cls, port, category: 'item' });
const restore = (items: ReturnType<typeof item>[]) =>
  ({ kind: 'loadout_restore', items }) as const;

/** An old, large full restore — the kit that used to win forever. */
const OLD_BIG = {
  event_type: 'burst_summary',
  event_timestamp: '2026-07-01T10:00:00Z',
  payload: restore([
    item('butcher_helmet', 'Armor_Helmet'),
    item('body_01', 'Body_ItemPort'),
    item('fresnel_lmg', 'wep_primary'),
    item('boomtube', 'wep_stowed_1'),
    item('mk4_grenade', 'grenade_attach_1'),
  ]),
};

/** A newer full restore, smaller — the kit the reader is actually wearing. */
const NEW_COMPLETE = {
  event_type: 'burst_summary',
  event_timestamp: '2026-08-27T17:17:10Z',
  payload: restore([
    item('odyssey_undersuit', 'Armor_Undersuit'),
    item('odyssey_helmet', 'Armor_Helmet'),
    item('klwe_pistol', 'wep_sidearm'),
  ]),
};

/** The newest burst, but only a weapon swap — must NOT replace a full kit. */
const NEWEST_PARTIAL = {
  event_type: 'burst_summary',
  event_timestamp: '2026-08-27T17:20:00Z',
  payload: restore([
    item('klwe_pistol_mag', 'magazine_attach_1'),
    item('klwe_pistol_mag', 'magazine_attach_2'),
    item('klwe_pistol', 'wep_sidearm'),
  ]),
};

test('isCompleteRestore separates a spawn restore from a re-equip', () => {
  // The body and the undersuit are only re-attached on a full restore.
  expect(isCompleteRestore(OLD_BIG.payload)).toBe(true);
  expect(isCompleteRestore(NEW_COMPLETE.payload)).toBe(true);
  expect(isCompleteRestore(NEWEST_PARTIAL.payload)).toBe(false);
});

test('a newer complete restore beats an older larger one', () => {
  // THE BUG. `pickFullestLoadoutBurst` returned OLD_BIG here (5 items vs 3),
  // so a kit from July kept showing after the August respawn — and with no
  // date on the page it read as current.
  const chosen = pickLoadoutBurst([NEWEST_PARTIAL, NEW_COMPLETE, OLD_BIG]);
  expect(chosen).toBe(NEW_COMPLETE);
});

test('a partial re-equip never displaces a complete restore', () => {
  // The reason the old rule existed: swapping a magazine must not blank the
  // paperdoll. Anchoring on completeness keeps that guarantee without
  // freezing the page on the biggest burst ever recorded.
  const chosen = pickLoadoutBurst([NEWEST_PARTIAL, NEW_COMPLETE]);
  expect(chosen).toBe(NEW_COMPLETE);
});

test('falls back to the fullest when nothing is a complete restore', () => {
  const smaller = {
    event_type: 'burst_summary',
    event_timestamp: '2026-08-28T00:00:00Z',
    payload: restore([item('a'), item('b')]),
  };
  expect(pickLoadoutBurst([smaller, NEWEST_PARTIAL])).toBe(NEWEST_PARTIAL);
});

test('an explicitly requested snapshot wins over the default choice', () => {
  const chosen = pickLoadoutBurst(
    [NEWEST_PARTIAL, NEW_COMPLETE, OLD_BIG],
    '2026-07-01T10:00:00Z',
  );
  expect(chosen).toBe(OLD_BIG);
});

test('an unknown requested snapshot falls back rather than rendering nothing', () => {
  const chosen = pickLoadoutBurst([NEWEST_PARTIAL, NEW_COMPLETE], '1999-01-01T00:00:00Z');
  expect(chosen).toBe(NEW_COMPLETE);
});

test('listLoadoutBursts returns every snapshot newest-first with its metadata', () => {
  const list = listLoadoutBursts([
    OLD_BIG,
    NEWEST_PARTIAL,
    { event_type: 'burst_summary', payload: { kind: 'terrain_load', items: [] } },
    { event_type: 'session_start', payload: {} },
    NEW_COMPLETE,
  ]);
  expect(list.map((s) => s.timestamp)).toEqual([
    '2026-08-27T17:20:00Z',
    '2026-08-27T17:17:10Z',
    '2026-07-01T10:00:00Z',
  ]);
  expect(list.map((s) => s.itemCount)).toEqual([3, 3, 5]);
  expect(list.map((s) => s.complete)).toEqual([false, true, true]);
});

test('pickLoadoutBurst returns undefined when no loadout burst exists', () => {
  expect(
    pickLoadoutBurst([
      { event_type: 'burst_summary', payload: { kind: 'terrain_load', items: [] } },
      { event_type: 'session_start', payload: {} },
    ]),
  ).toBeUndefined();
});
