import { describe, expect, it } from 'vitest';
import { assembleRule, kvToMap, type FormState } from './inference-rule';

describe('kvToMap', () => {
  it('drops rows with an empty (or whitespace-only) key', () => {
    expect(
      kvToMap([
        { key: 'who', value: 'Some_Guy' },
        { key: '', value: 'ignored' },
        { key: '   ', value: 'also ignored' },
      ]),
    ).toEqual({ who: 'Some_Guy' });
  });

  it('trims keys but preserves values verbatim', () => {
    expect(kvToMap([{ key: '  ship  ', value: '  DRAK_Cutlass_Black  ' }])).toEqual({
      ship: '  DRAK_Cutlass_Black  ',
    });
  });

  it('returns an empty object for no rows', () => {
    expect(kvToMap([])).toEqual({});
  });
});

describe('assembleRule', () => {
  it('assembles a full form state into the InferenceRuleDto shape', () => {
    const state: FormState = {
      id: '  combat.kill_streak  ',
      confidence: '0.75',
      window_secs: '30',
      trigger: {
        event_type: 'vehicle_destruction',
        field_equals: [
          { key: 'zone', value: 'OOC' },
          { key: '', value: 'dropped' },
        ],
      },
      followups: [
        {
          event_type: 'player_death',
          field_equals: [{ key: 'who', value: 'Some_Guy' }],
        },
        {
          event_type: 'resolve_spawn',
          field_equals: [],
        },
      ],
      emit: {
        event_type: 'combat.kill_streak',
        fields: [
          { key: 'timestamp', value: '${trigger.timestamp}' },
          { key: 'who', value: '${followups.0.who}' },
        ],
      },
    };

    expect(assembleRule(state)).toEqual({
      id: 'combat.kill_streak',
      confidence: 0.75,
      window_secs: 30,
      trigger: {
        event_type: 'vehicle_destruction',
        field_equals: { zone: 'OOC' },
      },
      followups: [
        {
          event_type: 'player_death',
          field_equals: { who: 'Some_Guy' },
        },
        {
          event_type: 'resolve_spawn',
          field_equals: {},
        },
      ],
      emits: {
        event_type: 'combat.kill_streak',
        fields: {
          timestamp: '${trigger.timestamp}',
          who: '${followups.0.who}',
        },
      },
    });
  });

  it('coerces confidence and window_secs to numbers', () => {
    const state: FormState = {
      id: 'x',
      confidence: '0.5',
      window_secs: '120',
      trigger: { event_type: 'a', field_equals: [] },
      followups: [],
      emit: { event_type: 'b', fields: [] },
    };
    const assembled = assembleRule(state);
    expect(assembled.confidence).toBe(0.5);
    expect(typeof assembled.confidence).toBe('number');
    expect(assembled.window_secs).toBe(120);
    expect(typeof assembled.window_secs).toBe('number');
  });
});
