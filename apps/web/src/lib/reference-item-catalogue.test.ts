import { describe, it, expect } from 'vitest';
import { catalogueDisplayName, isCatalogueItemType } from './reference-types';

describe('isCatalogueItemType', () => {
  it('keeps equipment a player owns, compares or flies with', () => {
    for (const t of [
      'WeaponPersonal', 'Char_Armor_Helmet', 'Shield', 'QuantumDrive',
      'PowerPlant', 'Cooler', 'Radar', 'FuelTank', 'Turret', 'Paints',
      'Container', 'InventoryContainer', 'Cargo', 'Food',
    ]) {
      expect(isCatalogueItemType(t), t).toBe(true);
    }
  });

  it('drops ship fixtures and engine plumbing', () => {
    // Each of these arrived with a duplicate placeholder name: 837 "Seat",
    // 249 "TRGT. STATUS", 138 "Bed", 81 "Access", 56 "Weapon Rack".
    for (const t of [
      'Seat', 'SeatAccess', 'SeatDashboard', 'Usable', 'Door', 'Elevator',
      'Display', 'ControlPanel', 'Relay', 'AttachedPart', 'DoorController',
    ]) {
      expect(isCatalogueItemType(t), t).toBe(false);
    }
  });

  it('matches case-insensitively and keeps an untyped row', () => {
    expect(isCatalogueItemType('seataccess')).toBe(false);
    expect(isCatalogueItemType('SEAT')).toBe(false);
    // A row the wiki never typed is kept: dropping it would hide real gear.
    expect(isCatalogueItemType(null)).toBe(true);
    expect(isCatalogueItemType(undefined)).toBe(true);
    expect(isCatalogueItemType('')).toBe(true);
  });
});

describe('catalogueDisplayName', () => {
  it('leaves a name the wiki actually wrote alone', () => {
    expect(catalogueDisplayName('BEHR_BallisticCannon_S4', 'C-788 Cannon')).toBe('C-788 Cannon');
    expect(catalogueDisplayName('ORIG_100i', '100i')).toBe('100i');
    // Punctuation and digits are not evidence of an engine token.
    expect(catalogueDisplayName('x', 'SW16BR1 “Buzzsaw” Repeater')).toBe('SW16BR1 “Buzzsaw” Repeater');
  });

  it('prettifies a row the wiki left as an engine identifier', () => {
    expect(catalogueDisplayName('ARMR_RSI_Lynx', 'armr_rsi_lynx')).toBe('Armr Rsi Lynx');
    // The heuristic expands a known manufacturer code: AEGS -> Aegis.
    expect(catalogueDisplayName('AEGS_Reclaimer_Salvage_Arm', 'aegs_reclaimer_salvage_arm'))
      .toBe('Aegis Reclaimer Salvage Arm');
    // Echoed back verbatim without underscores still counts.
    expect(catalogueDisplayName('Jumptown', 'jumptown')).toBe('Jumptown');
  });

  it('falls back to the class name when there is no display name at all', () => {
    expect(catalogueDisplayName('ARMR_RSI_Lynx', '')).toBe('Armr Rsi Lynx');
    expect(catalogueDisplayName('ARMR_RSI_Lynx', null)).toBe('Armr Rsi Lynx');
    expect(catalogueDisplayName('ARMR_RSI_Lynx', '   ')).toBe('Armr Rsi Lynx');
  });
});
