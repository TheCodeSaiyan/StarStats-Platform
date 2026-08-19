import { describe, it, expect } from 'vitest';
import {
  humanTitleForEntry,
  prettifySummary,
  type PrettyLookup,
} from './format';

const LOOKUP: PrettyLookup = new Map([
  ['aegs_avenger_stalker', 'Aegis Avenger Stalker'],
  ['drak_cutlass_black', 'Drake Cutlass Black'],
  ['klwe_lasercannon_s2', 'Klaus & Werner Sledge II'],
  ['ooc_stanton_2_crusader', 'Crusader'],
]);

describe('prettifySummary', () => {
  it('replaces a class-name token with the display name', () => {
    expect(prettifySummary('Vehicle destroyed: AEGS_Avenger_Stalker', LOOKUP))
      .toBe('Vehicle destroyed: Aegis Avenger Stalker');
  });

  it('replaces multiple tokens in one string', () => {
    const raw = 'Quantum target selected: AEGS_Avenger_Stalker → OOC_Stanton_2_Crusader';
    expect(prettifySummary(raw, LOOKUP))
      .toBe('Quantum target selected: Aegis Avenger Stalker → Crusader');
  });

  it('leaves unknown tokens unchanged', () => {
    expect(prettifySummary('Vehicle destroyed: ZZZZ_Unknown_Ship', LOOKUP))
      .toBe('Vehicle destroyed: ZZZZ_Unknown_Ship');
  });

  it('leaves strings without class-name tokens alone', () => {
    expect(prettifySummary('Near planet/moon: Crusader', LOOKUP))
      .toBe('Near planet/moon: Crusader');
    expect(prettifySummary('Server transition: starting', LOOKUP))
      .toBe('Server transition: starting');
  });

  it('is a no-op when the lookup is empty or undefined', () => {
    const raw = 'Vehicle destroyed: AEGS_Avenger_Stalker';
    expect(prettifySummary(raw, undefined)).toBe(raw);
    expect(prettifySummary(raw, new Map())).toBe(raw);
  });

  it('is case-insensitive on the lookup but preserves the original case in unmatched tokens', () => {
    // Lookup key is lowercase; input case shouldn't matter for the
    // match. If no match → the token's original case stays.
    expect(prettifySummary('hit: aegs_AVENGER_stalker', LOOKUP))
      .toBe('hit: aegs_AVENGER_stalker'); // doesn't match the regex (starts lowercase)
  });
});

describe('humanTitleForEntry with prettyLookup', () => {
  it('prettifies class-name tokens in the summary', () => {
    const out = humanTitleForEntry(
      {
        event_type: 'vehicle_destruction',
        summary: 'Vehicle destroyed: AEGS_Avenger_Stalker (level 0, by Self)',
      },
      LOOKUP,
    );
    expect(out).toBe('Vehicle destroyed: Aegis Avenger Stalker (level 0, by Self)');
  });

  it('falls back to the verb table when summary is the unparseable-payload sentinel', () => {
    const out = humanTitleForEntry(
      {
        event_type: 'vehicle_destruction',
        summary: 'vehicle_destruction (unparseable payload)',
      },
      LOOKUP,
    );
    // Verb-table entry wins; prettifier never sees it.
    expect(out).toBe('Ship destroyed');
  });

  it('works without a lookup (raw summary string)', () => {
    const raw = 'Vehicle destroyed: AEGS_Avenger_Stalker';
    expect(humanTitleForEntry({ event_type: 'vehicle_destruction', summary: raw }))
      .toBe(raw);
  });
});
