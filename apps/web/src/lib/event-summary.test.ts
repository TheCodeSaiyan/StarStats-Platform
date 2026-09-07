import { describe, expect, it } from 'vitest';
import { formatEventSummary } from './event-summary';

const ts = '2026-09-06T14:04:15.205Z';

describe('formatEventSummary: variants the tray emits that had no case', () => {
  it('quantum_arrived', () => {
    expect(
      formatEventSummary({
        type: 'quantum_arrived',
        timestamp: ts,
        vehicle_class: 'RSI_Polaris',
        vehicle_id: '1',
      }),
    ).toBe('Quantum travel complete in RSI Polaris');
  });

  it('quantum_route', () => {
    expect(
      formatEventSummary({
        type: 'quantum_route',
        timestamp: ts,
        start_system: 'Stanton',
        destination: 'Pyro',
        vehicle_class: 'RSI_Polaris',
        vehicle_id: '1',
      }),
    ).toBe('Route plotted: Stanton → Pyro in RSI Polaris');
  });

  it('location_changed with and without an origin', () => {
    expect(
      formatEventSummary({
        type: 'location_changed',
        timestamp: ts,
        from: 'Lorville',
        to: 'Everus Harbor',
      }),
    ).toBe('Moved from Lorville to Everus Harbor');
    expect(
      formatEventSummary({
        type: 'location_changed',
        timestamp: ts,
        from: null,
        to: 'Everus Harbor',
      }),
    ).toBe('Now at Everus Harbor');
  });

  it('shop_request_timed_out', () => {
    expect(
      formatEventSummary({
        type: 'shop_request_timed_out',
        timestamp: ts,
        shop_id: '1',
        item_class: null,
        timed_out_after_secs: 30,
      }),
    ).toBe('Shop request timed out after 30s');
  });

  it('mission_objective prefers the text and states the outcome', () => {
    expect(
      formatEventSummary({
        type: 'mission_objective',
        timestamp: ts,
        objective_id: 'obj-1',
        mission_id: null,
        state: 'completed',
        text: 'Deliver the package',
      }),
    ).toBe('Objective completed: Deliver the package');
    expect(
      formatEventSummary({
        type: 'mission_objective',
        timestamp: ts,
        objective_id: 'obj-1',
        mission_id: null,
        state: null,
        text: null,
      }),
    ).toBe('Objective: obj-1');
  });

  it('item_equip_change', () => {
    expect(
      formatEventSummary({
        type: 'item_equip_change',
        timestamp: ts,
        action: 'equip',
        item_class: 'scu_shirt_01_01_12',
        port: 'Clothing_Torso_0',
        items_count: null,
      }),
    ).toMatch(/^Equipped .+ \(Clothing_Torso_0\)$/);
    expect(
      formatEventSummary({
        type: 'item_equip_change',
        timestamp: ts,
        action: 'store',
        item_class: 'scu_shirt_01_01_12',
        port: null,
        items_count: null,
      }),
    ).toMatch(/^Stored /);
  });

  it('mission beacon events', () => {
    expect(
      formatEventSummary({
        type: 'mission_quantum_destination_selected',
        timestamp: ts,
        beacon_id: 'b',
        is_mission_destination: true,
        travel_confirmed: true,
      }),
    ).toBe('Mission destination selected, travel confirmed');
    expect(
      formatEventSummary({
        type: 'travel_to_contract_location',
        timestamp: ts,
        beacon_id: 'b',
        travel_started: true,
        travel_completed: false,
      }),
    ).toBe('Heading to the contract location');
    expect(
      formatEventSummary({
        type: 'travel_to_contract_location',
        timestamp: ts,
        beacon_id: 'b',
        travel_started: true,
        travel_completed: true,
      }),
    ).toBe('Arrived at the contract location');
  });
});

describe('formatEventSummary: what the live log showed', () => {
  const guid = '72b91153-5a3e-4d71-af5c-f6c57ea2891a';

  it('never prints a shop item that is only an engine GUID', () => {
    expect(
      formatEventSummary({ type: 'shop_buy_request', timestamp: ts, shop_id: '1', item_class: guid, quantity: null, raw: '' }),
    ).toBe('Shop purchase requested');
    expect(
      formatEventSummary({ type: 'shop_request_timed_out', timestamp: ts, shop_id: '1', item_class: guid, timed_out_after_secs: 30 }),
    ).toBe('Shop request timed out after 30s');
    expect(
      formatEventSummary({ type: 'shop_buy_request', timestamp: ts, shop_id: '1', item_class: 'scu_shirt_01', quantity: null, raw: '' }),
    ).toMatch(/^Buying /);
  });

  it('describes a stow without the engine vehicle id', () => {
    expect(
      formatEventSummary({
        type: 'vehicle_stowed',
        timestamp: ts,
        vehicle_id: '816633546929',
        landing_area: 'LandingArea_ShipElevator_HangarMediumFront',
        landing_area_id: '1',
        zone_host_id: '2',
      }),
    ).toBe('Stowed a ship at Ship Elevator Hangar Medium Front');
  });
});
