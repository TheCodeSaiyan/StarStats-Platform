import { describe, expect, it } from 'vitest';
import { parseFieldsInput } from './fields';

describe('parseFieldsInput', () => {
  it('splits on commas', () => {
    expect(parseFieldsInput('who,what,when')).toEqual(['who', 'what', 'when']);
  });

  it('splits on newlines', () => {
    expect(parseFieldsInput('who\nwhat\nwhen')).toEqual([
      'who',
      'what',
      'when',
    ]);
  });

  it('splits on a mix of commas and newlines', () => {
    expect(parseFieldsInput('who, what\nwhen')).toEqual([
      'who',
      'what',
      'when',
    ]);
  });

  it('trims whitespace around each field', () => {
    expect(parseFieldsInput('  who ,  what  \n  when  ')).toEqual([
      'who',
      'what',
      'when',
    ]);
  });

  it('drops empty entries from repeated separators / trailing commas', () => {
    expect(parseFieldsInput('who,,what,\n\nwhen,')).toEqual([
      'who',
      'what',
      'when',
    ]);
  });

  it('returns an empty array for blank input', () => {
    expect(parseFieldsInput('')).toEqual([]);
    expect(parseFieldsInput('   \n  ,  ')).toEqual([]);
  });
});
