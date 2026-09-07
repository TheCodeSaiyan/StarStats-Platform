import { describe, expect, it } from 'vitest';
import { formatEventType } from './event-types';

describe('formatEventType covers every variant the tray emits', () => {
  // These five were live in a 320k-event tray database on 2026-09-07 and had
  // no entry, so they rendered through the title-case fallback as `system`.
  it.each([
    ['quantum_arrived', 'travel'],
    ['location_changed', 'travel'],
    ['mission_quantum_destination_selected', 'mission'],
    ['travel_to_contract_location', 'mission'],
    ['shop_request_timed_out', 'commerce'],
  ])('%s is curated, not a fallback', (raw, group) => {
    const meta = formatEventType(raw);
    expect(meta.group).toBe(group);
    expect(meta.glyph).not.toBe('•');
  });
});
